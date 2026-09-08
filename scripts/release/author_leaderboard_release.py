#!/usr/bin/env python3
"""Author source-bound leaderboard release handoffs without deploying them.

The tool deliberately stops at immutable local artifacts.  It has no SSH,
Cloudflare API, service-manager, or production-filesystem authority.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import tomllib
from typing import Any, Iterable


DIGEST = re.compile(r"^[0-9a-f]{64}$")
COMMIT = re.compile(r"^[0-9a-f]{40}$")
TREE = re.compile(r"^[0-9a-f]{40}$")
INSTALL_ROOT = Path("/home/robinhood/.local/opt/robin-highscores")
STATE_ROOT = Path("/home/robinhood/.local/share/robin-highscores")
CAMPAIGN_MEDIA_TYPE = "application/x-robin-campaign+bitcode"
DATABASE_SCHEMA_SOURCE = Path("crates/robin_run_protocol/src/lib.rs")
DATABASE_SCHEMA_PATTERN = re.compile(
    r"^pub const HIGHSCORES_DATABASE_SCHEMA_VERSION: i64 = ([0-9]+);$", re.MULTILINE
)

REGISTRY_DIRECTORIES = (
    "builds",
    "content-manifests",
    "campaign-content-manifests",
    "rules-configs",
    "ruleset-manifests",
    "published-rulesets",
    "competitions",
    "policies",
)
BINARY_SPECS = (
    ("admin", "robin-highscores-admin", "application/x-executable"),
    ("manifest_tool", "robin-highscores-manifestctl", "application/x-executable"),
    ("server", "robin-highscores-server", "application/x-executable"),
    ("worker", "robin-highscores-worker", "application/x-executable"),
    (
        "replay_verifier",
        "robin-replay-verifier",
        "application/vnd.robinhood.ranked-replay-verifier-v2",
    ),
)
CONFIG_SPECS = (
    ("server", "highscores-server.toml", "application/toml"),
    ("worker", "highscores-worker.toml", "application/toml"),
    ("api_environment", "api.env", "text/plain; charset=utf-8"),
    ("worker_environment", "worker.env", "text/plain; charset=utf-8"),
)
HOST_SPECS = (
    ("user_target", "robin-highscores.target", "systemd/user/robin-highscores.target", 0o440),
    ("api_service", "robin-highscores-api.service", "systemd/user/robin-highscores-api.service", 0o440),
    ("worker_service", "robin-highscores-worker.service", "systemd/user/robin-highscores-worker.service", 0o440),
    ("backup_service", "robin-highscores-backup.service", "systemd/user/robin-highscores-backup.service", 0o440),
    ("backup_timer", "robin-highscores-backup.timer", "systemd/user/robin-highscores-backup.timer", 0o440),
    ("deploy_release_script", "deploy-release.sh", "deploy/deploy-release.sh", 0o550),
    ("rollback_release_script", "rollback-release.sh", "deploy/rollback-release.sh", 0o550),
    ("validate_release_script", "validate-release-bundle.sh", "deploy/validate-release-bundle.sh", 0o550),
    (
        "real_runtime_fence_release_gate",
        "tests/real-runtime-fence-release-gate.sh",
        "deploy/tests/real-runtime-fence-release-gate.sh",
        0o550,
    ),
    (
        "real_runtime_fence_harness",
        "tests/real-runtime-fence-e2e.py",
        "deploy/tests/real-runtime-fence-e2e.py",
        0o550,
    ),
    (
        "real_runtime_fence_selftest",
        "tests/real-runtime-fence-e2e-selftest.py",
        "deploy/tests/real-runtime-fence-e2e-selftest.py",
        0o550,
    ),
    ("root_once_script", "root-once.sh", "deploy/root-once.sh", 0o550),
    ("nginx_challenge", "nginx-robinhood-api.challenge.conf", "deploy/nginx-robinhood-api.challenge.conf", 0o440),
    ("nginx_cloudflare_only", "nginx-robinhood-cloudflare-only.conf", "deploy/nginx-robinhood-cloudflare-only.conf", 0o440),
    ("nginx_api_locations", "nginx-robinhood-api.locations.conf", "deploy/nginx-robinhood-api.locations.conf", 0o440),
    ("nginx_vhost", "nginx-robinhood-api.vhost.conf", "deploy/nginx-robinhood-api.vhost.conf", 0o440),
    ("deployment_readme", "../README.md", "deploy/README.md", 0o440),
    ("operator_runbook", "../README.md", "deploy/VPS_RELEASE_INSTALL.md", 0o440),
    ("backup_runbook", "../README.md", "deploy/BACKUP_RESTORE.md", 0o440),
)


class AuthoringError(RuntimeError):
    """An authority or filesystem invariant failed."""


def ensure(condition: bool, message: str) -> None:
    if not condition:
        raise AuthoringError(message)


def canonical_bytes(value: Any) -> bytes:
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode()


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def require_digest(value: Any, label: str) -> str:
    ensure(isinstance(value, str) and DIGEST.fullmatch(value) is not None, f"invalid {label}")
    ensure(value != "0" * 64, f"zero {label}")
    return value


def require_commit(value: Any, label: str = "source commit") -> str:
    ensure(isinstance(value, str) and COMMIT.fullmatch(value) is not None, f"invalid {label}")
    return value


def exact_keys(value: Any, expected: Iterable[str], label: str) -> dict[str, Any]:
    ensure(isinstance(value, dict), f"{label} must be an object")
    expected_set = set(expected)
    ensure(set(value) == expected_set, f"{label} fields differ: {sorted(set(value) ^ expected_set)}")
    return value


def require_regular(path: Path, label: str, *, executable: bool | None = None) -> Path:
    try:
        metadata = path.lstat()
        resolved = path.resolve(strict=True)
    except FileNotFoundError as error:
        raise AuthoringError(f"missing {label}: {path}") from error
    ensure(path.is_absolute() and resolved == path, f"{label} is not an absolute normalized path: {path}")
    ensure(stat.S_ISREG(metadata.st_mode) and not path.is_symlink(), f"{label} is not a real regular file: {path}")
    ensure(metadata.st_nlink == 1, f"{label} is hard linked: {path}")
    if executable is True:
        ensure(metadata.st_mode & 0o111 != 0, f"{label} is not executable: {path}")
    if executable is False:
        ensure(metadata.st_mode & 0o111 == 0, f"{label} is unexpectedly executable: {path}")
    return path


def require_directory(path: Path, label: str) -> Path:
    try:
        metadata = path.lstat()
        resolved = path.resolve(strict=True)
    except FileNotFoundError as error:
        raise AuthoringError(f"missing {label}: {path}") from error
    ensure(stat.S_ISDIR(metadata.st_mode) and not path.is_symlink(), f"{label} is not a real directory: {path}")
    ensure(path.is_absolute() and resolved == path, f"{label} is not an absolute normalized path: {path}")
    return path


def artifact(path: Path, media_type: str) -> dict[str, Any]:
    require_regular(path, "artifact")
    return {
        "byte_length": path.stat().st_size,
        "media_type": media_type,
        "sha256": sha256_file(path),
    }


def checked_artifact(path: Path, expected: Any, label: str) -> dict[str, Any]:
    record = exact_keys(expected, ("byte_length", "media_type", "sha256"), f"{label} artifact")
    require_digest(record["sha256"], f"{label} digest")
    ensure(isinstance(record["byte_length"], int) and record["byte_length"] > 0, f"invalid {label} byte length")
    ensure(isinstance(record["media_type"], str) and record["media_type"], f"invalid {label} media type")
    actual = artifact(path, record["media_type"])
    ensure(actual == record, f"{label} differs from its artifact authority")
    return record


def checked_binding(value: Any, label: str, *, executable: bool = False) -> tuple[Path, dict[str, Any]]:
    binding = exact_keys(value, ("artifact", "source"), f"{label} binding")
    ensure(isinstance(binding["source"], str), f"{label} source must be a path string")
    source = require_regular(Path(binding["source"]), label, executable=executable)
    return source, checked_artifact(source, binding["artifact"], label)


def load_json(path: Path, label: str, *, canonical: bool = True) -> tuple[Any, bytes]:
    require_regular(path, label)
    data = path.read_bytes()
    try:
        value = json.loads(data)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise AuthoringError(f"invalid {label}: {path}: {error}") from error
    if canonical:
        ensure(data == canonical_bytes(value), f"{label} is not compact canonical JSON: {path}")
    return value, data


def write_new(path: Path, data: bytes, mode: int = 0o600) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, mode)
    try:
        with os.fdopen(descriptor, "wb", closefd=False) as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
    except BaseException:
        path.unlink(missing_ok=True)
        raise
    finally:
        os.close(descriptor)


def write_json(path: Path, value: Any, mode: int = 0o600) -> None:
    write_new(path, canonical_bytes(value), mode)


def install_directory_no_replace(stage: Path, output: Path) -> None:
    identity = (stage.stat().st_dev, stage.stat().st_ino)
    completed = subprocess.run(
        ["/usr/bin/mv", "--no-clobber", "--no-target-directory", str(stage), str(output)],
        text=True,
        capture_output=True,
    )
    ensure(completed.returncode == 0, f"atomic output install failed: {completed.stderr}")
    ensure(not stage.exists(), "output appeared concurrently; staging tree was not installed")
    installed = output.lstat()
    ensure(stat.S_ISDIR(installed.st_mode) and not output.is_symlink(), "installed output is not a real directory")
    ensure((installed.st_dev, installed.st_ino) == identity, "installed output is not the authored staging directory")
    descriptor = os.open(output.parent, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def cleanup_authored_stage(stage: Path) -> None:
    """Remove one exact retained staging tree without following symlinks."""
    try:
        metadata = stage.lstat()
    except FileNotFoundError:
        return
    ensure(
        stage.is_absolute() and stat.S_ISDIR(metadata.st_mode) and not stage.is_symlink(),
        "refusing to clean an unsafe staging root",
    )
    ensure(shutil.rmtree.avoids_symlink_attacks, "platform lacks descriptor-safe tree removal")
    for directory, child_directories, _files in os.walk(stage, topdown=False, followlinks=False):
        parent = Path(directory)
        for name in child_directories:
            child = parent / name
            metadata = child.lstat()
            if stat.S_ISDIR(metadata.st_mode) and not child.is_symlink():
                os.chmod(child, 0o700, follow_symlinks=False)
        os.chmod(parent, 0o700, follow_symlinks=False)
    shutil.rmtree(stage)


def run(arguments: list[str], *, cwd: Path | None = None) -> str:
    completed = subprocess.run(arguments, cwd=cwd, text=True, capture_output=True)
    if completed.returncode != 0:
        raise AuthoringError(
            f"command failed ({completed.returncode}): {' '.join(arguments)}\n"
            f"stdout:\n{completed.stdout}\nstderr:\n{completed.stderr}"
        )
    return completed.stdout.strip()


def git_environment() -> dict[str, str]:
    environment = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
    environment.update(
        {
            "GIT_CONFIG_GLOBAL": "/dev/null",
            "GIT_CONFIG_NOSYSTEM": "1",
            "GIT_NO_REPLACE_OBJECTS": "1",
            "LC_ALL": "C",
        }
    )
    return environment


def git_bytes(repo: Path, *arguments: str) -> bytes:
    completed = subprocess.run(
        [
            "/usr/bin/git",
            "--no-replace-objects",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
            "-C",
            str(repo),
            *arguments,
        ],
        capture_output=True,
        env=git_environment(),
    )
    if completed.returncode != 0:
        raise AuthoringError(
            f"Git identity command failed ({completed.returncode}): {' '.join(arguments)}\n"
            f"stdout:\n{completed.stdout.decode(errors='replace')}\n"
            f"stderr:\n{completed.stderr.decode(errors='replace')}"
        )
    return completed.stdout


def git(repo: Path, *arguments: str) -> str:
    return git_bytes(repo, *arguments).decode("utf-8").strip()


def load_source_authority(path: Path, repo: Path) -> dict[str, Any]:
    authority, _ = load_json(path, "source authority")
    authority = exact_keys(
        authority,
        ("cargo_lock_sha256", "database_schema_version", "schema_version", "source_commit", "source_tree_sha1"),
        "source authority",
    )
    ensure(authority["schema_version"] == 1, "unsupported source-authority schema")
    commit = require_commit(authority["source_commit"])
    ensure(isinstance(authority["source_tree_sha1"], str) and TREE.fullmatch(authority["source_tree_sha1"]), "invalid source tree")
    require_digest(authority["cargo_lock_sha256"], "Cargo.lock digest")
    ensure(isinstance(authority["database_schema_version"], int) and authority["database_schema_version"] > 0, "invalid database schema")
    require_directory(repo, "source repository")
    ensure(git(repo, "rev-parse", "--show-toplevel") == str(repo), "Git worktree root differs from source repository")
    ensure(git(repo, "for-each-ref", "--format=%(refname)", "refs/replace") == "", "source repository contains forbidden replacement refs")
    ensure(git(repo, "rev-parse", "HEAD") == commit, "repository HEAD differs from source authority")
    ensure(git(repo, "rev-parse", "HEAD^{tree}") == authority["source_tree_sha1"], "repository tree differs from source authority")
    ensure(git(repo, "status", "--porcelain", "--untracked-files=no") == "", "repository has tracked modifications")
    ensure(sha256_file(require_regular(repo / "Cargo.lock", "Cargo.lock")) == authority["cargo_lock_sha256"], "Cargo.lock differs from source authority")
    schema_source = require_regular(repo / DATABASE_SCHEMA_SOURCE, "database schema source")
    match = DATABASE_SCHEMA_PATTERN.search(schema_source.read_text(encoding="utf-8"))
    ensure(match is not None, "compiled database schema constant is absent or ambiguous")
    ensure(len(DATABASE_SCHEMA_PATTERN.findall(schema_source.read_text(encoding="utf-8"))) == 1, "compiled database schema constant repeats")
    ensure(int(match.group(1)) == authority["database_schema_version"], "database schema differs from source authority")
    return authority


def load_binary_authority(path: Path, source: dict[str, Any]) -> list[dict[str, Any]]:
    authority, _ = load_json(path, "MUSL binary authority")
    authority = exact_keys(
        authority,
        ("binaries", "cargo_lock_sha256", "schema_version", "source_commit", "source_tree_sha1"),
        "MUSL binary authority",
    )
    ensure(authority["schema_version"] == 2, "MUSL binary authority requires schema 2")
    for field in ("cargo_lock_sha256", "source_commit", "source_tree_sha1"):
        ensure(authority[field] == source[field], f"MUSL binary authority {field} differs from source authority")
    entries = authority["binaries"]
    ensure(isinstance(entries, list), "MUSL binaries must be an array")
    ensure([entry.get("role") for entry in entries if isinstance(entry, dict)] == [item[0] for item in BINARY_SPECS], "MUSL binary role order differs")
    checked: list[dict[str, Any]] = []
    identities: set[str] = set()
    for entry, (role, name, media_type) in zip(entries, BINARY_SPECS, strict=True):
        entry = exact_keys(entry, ("artifact", "role", "source"), f"{role} binary")
        ensure(entry["role"] == role and isinstance(entry["source"], str), f"{role} binary binding differs")
        local = require_regular(Path(entry["source"]), f"{role} binary", executable=True)
        ensure(local.name == name, f"{role} binary filename differs")
        exact = checked_artifact(local, entry["artifact"], f"{role} binary")
        ensure(exact["media_type"] == media_type, f"{role} binary media type differs")
        ensure(exact["sha256"] not in identities, "one MUSL binary was substituted for another role")
        identities.add(exact["sha256"])
        checked.append({"artifact": exact, "local": local, "role": role})
    return checked


def author_source_authority(args: argparse.Namespace) -> None:
    authority = {
        "cargo_lock_sha256": require_digest(args.cargo_lock_sha256, "Cargo.lock digest"),
        "database_schema_version": args.database_schema_version,
        "schema_version": 1,
        "source_commit": require_commit(args.source_commit),
        "source_tree_sha1": args.source_tree_sha1,
    }
    ensure(TREE.fullmatch(authority["source_tree_sha1"]) is not None, "invalid source tree")
    ensure(isinstance(authority["database_schema_version"], int) and authority["database_schema_version"] > 0, "invalid database schema")
    output = Path(args.output)
    ensure(output.is_absolute() and not output.exists(), "source-authority output must be an absent absolute path")
    write_json(output, authority)
    try:
        load_source_authority(output, Path(args.repo).resolve(strict=True))
    except BaseException:
        output.unlink(missing_ok=True)
        raise
    print(sha256_file(output))


def addressed_documents(root: Path, name: str, *, published: bool = False) -> dict[str, dict[str, Any]]:
    directory = require_directory(root / name, f"{name} registry")
    result: dict[str, dict[str, Any]] = {}
    for path in sorted(directory.iterdir(), key=lambda item: item.name):
        ensure(path.suffix == ".json", f"unexpected non-JSON {name} entry: {path.name}")
        identity = require_digest(path.stem, f"{name} filename")
        document, data = load_json(path, f"{name} document")
        ensure(isinstance(document, dict), f"{name} document is not an object")
        if published:
            ensure(document.get("ruleset_manifest_sha256") == identity, f"published-rulesets filename identity differs")
        else:
            ensure(sha256_bytes(data) == identity, f"{name} filename does not hash its bytes")
        ensure(identity not in result, f"duplicate {name} identity")
        result[identity] = document
    return result


def subject_key(value: Any) -> tuple[str, str]:
    subject = exact_keys(value, ("kind", "mission_id"), "content subject")
    ensure(subject["kind"] in ("field_mission", "headquarters"), "unknown content subject kind")
    ensure(isinstance(subject["mission_id"], str) and subject["mission_id"], "empty mission ID")
    return subject["kind"], subject["mission_id"]


def inspect_registry(root: Path, source_commit: str, run_preflight_key: str) -> dict[str, Any]:
    require_directory(root, "manifest registry")
    ensure(sorted(item.name for item in root.iterdir()) == sorted(REGISTRY_DIRECTORIES), "manifest registry shape differs")
    builds = addressed_documents(root, "builds")
    content = addressed_documents(root, "content-manifests")
    campaigns = addressed_documents(root, "campaign-content-manifests")
    rules_configs = addressed_documents(root, "rules-configs")
    immutable = addressed_documents(root, "ruleset-manifests")
    published = addressed_documents(root, "published-rulesets", published=True)
    competitions = addressed_documents(root, "competitions")
    policies = addressed_documents(root, "policies")
    ensure(len(builds) == 1, "registry must contain exactly one BuildManifestV2")
    ensure(content and campaigns and rules_configs and immutable and policies, "manifest registry is incomplete")
    ensure(not competitions, "initial release lane must not contain competitions")
    ensure(set(immutable) == set(published), "immutable and published ruleset identities differ")
    build_digest, build = next(iter(builds.items()))
    ensure(build.get("schema_version") == 2 and build.get("source_commit") == source_commit, "BuildManifestV2 source differs")
    verifier = build.get("verifier")
    ensure(isinstance(verifier, dict) and isinstance(verifier.get("artifact"), dict), "build omits verifier")
    verifier_sha256 = require_digest(verifier["artifact"].get("sha256"), "verifier digest")

    by_edition: dict[str, dict[tuple[str, str], tuple[str, dict[str, Any]]]] = {"demo": {}, "full": {}}
    for identity, document in content.items():
        edition = document.get("edition")
        ensure(edition in by_edition, "content has unknown edition")
        key = subject_key(document.get("subject"))
        ensure(key not in by_edition[edition], "content subject repeats")
        by_edition[edition][key] = (identity, document)
    ensure(all(by_edition.values()), "content omits an edition")
    for edition in by_edition:
        by_edition[edition] = dict(sorted(by_edition[edition].items()))

    campaign_by_edition: dict[str, tuple[str, dict[str, Any]]] = {}
    for identity, document in campaigns.items():
        edition = document.get("edition")
        ensure(edition in by_edition and edition not in campaign_by_edition, "campaign catalog edition differs")
        entries = document.get("entries")
        ensure(isinstance(entries, list), "campaign catalog entries are absent")
        actual = {(subject_key(entry.get("subject")), entry.get("content_manifest_sha256")) for entry in entries}
        expected = {(key, record[0]) for key, record in by_edition[edition].items()}
        ensure(actual == expected and len(entries) == len(expected), f"{edition} campaign catalog differs from content")
        campaign_by_edition[edition] = (identity, document)
    ensure(set(campaign_by_edition) == {"demo", "full"}, "campaign catalogs omit Demo or Full")

    rulesets: dict[tuple[str, str], tuple[str, dict[str, Any]]] = {}
    for identity, status in published.items():
        ensure(status.get("manifest") == immutable[identity], "published ruleset embeds a substituted manifest")
        ensure(status.get("operational_status") == {"status": "active"}, "initial rulesets must be active")
        manifest = immutable[identity]
        config = require_digest(manifest.get("rules_config_sha256"), "rules config")
        ensure(config in rules_configs, "ruleset references an absent rules config")
        requirement = manifest.get("canonical_campaign_state")
        ensure(isinstance(requirement, dict), "ruleset omits campaign-state requirement")
        edition = requirement.get("edition")
        ensure(edition in by_edition and requirement.get("rules_config_sha256") == config, "ruleset campaign requirement differs")
        expected_kind = "individual_template" if edition == "demo" else "full_campaign_genesis"
        ensure(requirement.get("kind") == expected_kind, "ruleset campaign-state kind differs")
        ensure(manifest.get("run_preflight_grant_public_key") == run_preflight_key, "ruleset preflight authority differs")
        ensure(manifest.get("allowed_build_manifest_sha256") == [build_digest], "ruleset build allowlist differs")
        ensure(manifest.get("allowed_content_manifest_sha256") == sorted(item[0] for item in by_edition[edition].values()), "ruleset content allowlist differs")
        expected_campaigns = [] if edition == "demo" else [campaign_by_edition["full"][0]]
        ensure(manifest.get("allowed_campaign_content_manifest_sha256") == expected_campaigns, "ruleset campaign allowlist differs")
        ensure((edition, config) not in rulesets, "edition/config ruleset repeats")
        rulesets[(edition, config)] = (identity, manifest)
    expected_rulesets = {(edition, config) for edition in ("demo", "full") for config in rules_configs}
    ensure(set(rulesets) == expected_rulesets, "rulesets do not cover every config in both editions")
    return {
        "build_digest": build_digest,
        "campaigns": campaign_by_edition,
        "content": by_edition,
        "rules_configs": rules_configs,
        "rulesets": rulesets,
        "verifier_sha256": verifier_sha256,
    }


def inspect_campaign_states(entries: Any, registry: dict[str, Any]) -> dict[tuple[str, str], dict[str, Any]]:
    ensure(isinstance(entries, list), "campaign_states must be an array")
    result: dict[tuple[str, str], dict[str, Any]] = {}
    for record in entries:
        record = exact_keys(record, ("artifact", "edition", "kind", "rules_config_sha256", "source"), "campaign state")
        edition = record["edition"]
        ensure(edition in ("demo", "full"), "campaign state edition differs")
        config = require_digest(record["rules_config_sha256"], "campaign rules config")
        expected_kind = "individual_template" if edition == "demo" else "full_campaign_genesis"
        ensure(record["kind"] == expected_kind, "campaign state kind differs")
        source = Path(record["source"])
        require_directory(source.parent.resolve(strict=True), "campaign source parent")
        checked_artifact(source, record["artifact"], "campaign state")
        ensure(record["artifact"]["media_type"] == CAMPAIGN_MEDIA_TYPE, "campaign media type differs")
        key = (edition, config)
        ensure(key in registry["rulesets"] and key not in result, "campaign state matrix differs")
        result[key] = record
    ensure(set(result) == set(registry["rulesets"]), "campaign states do not cover every ruleset")
    return result


def toml_value(value: Any) -> str:
    if isinstance(value, str):
        return json.dumps(value, ensure_ascii=False)
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, int):
        return str(value)
    if isinstance(value, list):
        return "[" + ", ".join(toml_value(item) for item in value) + "]"
    if isinstance(value, dict):
        return "{ " + ", ".join(f"{key} = {toml_value(item)}" for key, item in value.items()) + " }"
    raise AuthoringError(f"unsupported TOML value: {value!r}")


def slug(value: str) -> str:
    return re.sub(r"[^a-z0-9]+", "-", value.lower()).strip("-") or "subject"


def profiles(registry: dict[str, Any], states: dict[tuple[str, str], dict[str, Any]], commit: str) -> list[dict[str, Any]]:
    release_root = INSTALL_ROOT / "releases" / commit
    result: list[dict[str, Any]] = []
    for (edition, config), (ruleset_identity, ruleset) in sorted(registry["rulesets"].items()):
        state = states[(edition, config)]
        for ordinal, (subject, (content_identity, content)) in enumerate(registry["content"][edition].items()):
            kind, mission = subject
            profile_id = f"{edition}-{ruleset['preset_id']}-{ruleset['difficulty_id']}-{ordinal:02d}-{slug(mission)}"
            allowed_scopes = ["individual_level"] if edition == "demo" else ["campaign_continuation"]
            if edition == "full" and kind == "field_mission" and mission == "H01_Lin_VL":
                allowed_scopes = ["campaign_genesis", "campaign_continuation"]
            profile: dict[str, Any] = {
                "id": profile_id,
                "content_subject": content["subject"],
                "mission_display_name": content["name"],
                "allowed_scopes": allowed_scopes,
                "build_manifest_id": registry["build_digest"],
                "content_manifest_id": content_identity,
                "config_id": config,
                "ruleset_id": ruleset_identity,
                "template_id": profile_id,
                "canonical_campaign_state": {
                    "requirement": {
                        "edition": edition,
                        "kind": state["kind"],
                        "rules_config_sha256": config,
                    },
                    "artifact": state["artifact"],
                },
                "canonical_campaign_state_path": str(release_root / "private/campaign-states" / state["artifact"]["sha256"]),
                "allowed_metrics": ruleset["metrics"],
                "ruleset_display_name": ruleset["display_name"],
                "preset_id": ruleset["preset_id"],
                "preset_name": ruleset["preset_name"],
                "difficulty_id": ruleset["difficulty_id"],
                "difficulty_name": ruleset["difficulty_name"],
                "build_display_name": f"Robin Hood verified {commit[:12]}",
                "viewer_engine_build": registry["build_digest"],
                "viewer_available": True,
                "viewer_content_requirement": "bundled_demo" if edition == "demo" else "user_local_retail",
            }
            if edition == "full":
                profile["campaign_content_manifest_id"] = registry["campaigns"]["full"][0]
            result.append(profile)
    ensure(len({item["id"] for item in result}) == len(result), "admission profile IDs repeat")
    return result


def render_server_config(admission_profiles: list[dict[str, Any]], commit: str) -> bytes:
    release_root = INSTALL_ROOT / "releases" / commit
    fields = (
        ("bind", "127.0.0.1:8787"),
        ("database_path", str(STATE_ROOT / "database/highscores.sqlite3")),
        ("runtime_fence_directory", str(STATE_ROOT / "runtime-fence")),
        ("replay_directory", str(STATE_ROOT / "replays")),
        ("campaign_state_directory", str(STATE_ROOT / "campaign-states")),
        ("cursor_secret_path", str(STATE_ROOT / "api-secrets/cursor-hmac.key")),
        ("competition_run_grant_secret_path", str(STATE_ROOT / "api-secrets/competition-run-grant.key")),
        ("run_preflight_grant_secret_path", str(STATE_ROOT / "api-secrets/run-preflight-grant.key")),
        ("backup_authority_hmac_secret_path", str(STATE_ROOT / "api-secrets/backup-authority-hmac.key")),
        ("moderation_bearer_token_path", str(STATE_ROOT / "api-secrets/moderation-bearer.token")),
        ("moderation_operator_id", "production-operator"),
        ("allowed_origins", []),
        ("trusted_proxy_cidrs", ["127.0.0.1/32", "::1/128"]),
        ("challenge_requests_per_minute_per_ip", 120),
        ("abuse_reports_per_hour_per_ip", 10),
        ("abuse_reports_per_hour_per_key", 25),
        ("abuse_reports_per_hour_per_target", 10),
        ("manifest_directory", str(release_root / "config/manifests")),
        ("competitions", []),
        ("max_replay_bytes", 16_777_216),
        ("max_campaign_bytes", 16_777_216),
        ("max_metadata_bytes", 65_536),
        ("max_pending_submissions", 10_000),
        ("max_concurrent_requests", 256),
        ("max_concurrent_uploads", 32),
        ("upload_timeout_seconds", 120),
        ("upload_reservation_ttl_seconds", 1_800),
        ("max_page_size", 100),
        ("challenge_ttl_seconds", 600),
        ("run_preflight_ttl_seconds", 86_400),
        ("database_busy_timeout_ms", 5_000),
        ("tombstone_retention_days", 30),
        ("rejected_replay_retention_hours", 24),
        ("orphan_replay_retention_hours", 24),
        ("minimum_storage_free_bytes", 1_073_741_824),
        ("backup_manifest_path", str(STATE_ROOT / "status/backup-status.json")),
        ("release_manifest_path", str(release_root / "vps-release-manifest-v2.json")),
        ("maximum_backup_age_hours", 32),
    )
    lines = ["# Deterministically authored production leaderboard configuration."]
    lines.extend(f"{key} = {toml_value(value)}" for key, value in fields)
    for profile in admission_profiles:
        lines.extend(("", "[[admission_profiles]]"))
        lines.extend(f"{key} = {toml_value(value)}" for key, value in profile.items())
    return ("\n".join(lines) + "\n").encode()


def render_worker_config(commit: str, catalog: str, verifier: str, source_manifests: dict[str, str], bwrap: str, prlimit: str) -> bytes:
    release_root = INSTALL_ROOT / "releases" / commit
    lines = [
        "# Deterministically authored production verifier-worker configuration.",
        f'server_config = "{release_root}/config/highscores-server.toml"',
        'worker_id = "worker-01"',
        f'campaign_state_directory = "{STATE_ROOT}/campaign-states"',
        f'verifier_job_config_catalog = "{release_root}/private/verifier/operator-config/{catalog}"',
        f'verifier_job_config_catalog_sha256 = "{catalog}"',
        f'demo_raw_content_manifest = "{release_root}/private/source-tree-manifests-v2/{source_manifests["demo"]}.json"',
        f'full_raw_content_manifest = "{release_root}/private/source-tree-manifests-v2/{source_manifests["full"]}.json"',
        "poll_interval_ms = 500",
        "lease_seconds = 210",
        "retry_seconds = 30",
        "max_verifier_attempts = 3",
        "",
        "[verifier_launcher]",
        'bwrap_program = "/usr/bin/bwrap"',
        f'bwrap_sha256 = "{bwrap}"',
        'prlimit_program = "/usr/bin/prlimit"',
        f'prlimit_sha256 = "{prlimit}"',
        f'verifier_program = "{release_root}/bin/robin-replay-verifier"',
        f'verifier_sha256 = "{verifier}"',
        "wall_timeout_seconds = 120",
        "cpu_limit_seconds = 120",
        "address_space_limit_bytes = 1073741824",
        "process_limit = 32",
        "open_files_limit = 128",
        "file_size_limit_bytes = 134217728",
        "max_request_bytes = 1048576",
        "",
        "[limits]",
        "max_input_bytes = 16777216",
        "max_compressed_bytes = 16777216",
        "max_decompressed_bytes = 67108864",
        "max_base64_payload_bytes = 16777216",
        "max_campaign_bytes = 67108864",
        "max_frames = 2000000",
        "max_version_bytes = 256",
        "max_mission_id_bytes = 128",
        "max_metadata_records = 4096",
        "max_entries_per_frame = 1024",
    ]
    return ("\n".join(lines) + "\n").encode()


def render_host_files(repo: Path, stage: Path, final: Path, commit: str) -> list[dict[str, Any]]:
    source_root = repo / "crates/robin_highscores/deploy"
    result: list[dict[str, Any]] = []
    for role, name, relative, sealed_mode in HOST_SPECS:
        # Bundle document roles share one tracked guide; no source redirects.
        source_path = source_root.parent / "README.md" if name == "../README.md" else source_root / name
        source = require_regular(source_path, "deployment source")
        tracked = git_bytes(repo, "show", f"{commit}:{source.relative_to(repo).as_posix()}")
        ensure(source.read_bytes() == tracked, f"deployment source differs from {commit}: {name}")
        text = tracked.decode("utf-8").replace("@SOURCE_COMMIT@", commit)
        ensure("@SOURCE_COMMIT@" not in text, f"unrendered source commit in {name}")
        # Preserve the V2 bundle layout without duplicating the complete guide.
        runbook_sections = {
            "operator_runbook": "vps-installation-and-rollback",
            "backup_runbook": "backup-and-disaster-recovery",
        }
        if role in runbook_sections:
            section = runbook_sections[role]
            text = f"See the [combined guide](README.md#{section}).\n"
        target = stage / relative
        ensure(sealed_mode in (0o440, 0o550), f"unsupported sealed host mode for {role}")
        write_new(target, text.encode(), sealed_mode | 0o200)
        if relative.endswith(".sh"):
            media_type = "application/x-sh"
        elif relative.endswith(".py"):
            media_type = "text/x-python; charset=utf-8"
        else:
            media_type = "text/plain; charset=utf-8"
        result.append({
            "artifact": artifact(target, media_type),
            "path": relative,
            "role": role,
            "source": str(final / relative),
        })
    return result


def source_manifest_identities(value: Any) -> dict[str, str]:
    mapping = exact_keys(value, ("demo", "full"), "source_tree_manifests")
    result: dict[str, str] = {}
    for edition in ("demo", "full"):
        path = Path(mapping[edition])
        document, data = load_json(path, f"{edition} source-tree manifest")
        identity = require_digest(path.stem, f"{edition} source-tree filename")
        ensure(sha256_bytes(data) == identity, f"{edition} source-tree filename differs from bytes")
        ensure(document.get("edition") == edition and document.get("source_format") == "loose_native_v1", f"{edition} source-tree authority differs")
        result[edition] = identity
    return result


def seal(root: Path) -> None:
    directories: list[Path] = []
    for directory, child_directories, files in os.walk(root, followlinks=False):
        child_directories.sort()
        files.sort()
        parent = Path(directory)
        directories.append(parent)
        ensure(not parent.is_symlink(), f"generated directory is a symlink: {parent}")
        for name in files:
            path = require_regular(parent / name, "generated file")
            mode = stat.S_IMODE(path.stat(follow_symlinks=False).st_mode)
            os.chmod(path, 0o550 if mode & 0o111 else 0o440)
    for directory in sorted(directories, key=lambda item: len(item.parts), reverse=True):
        os.chmod(directory, 0o550)


def author_configs(args: argparse.Namespace) -> None:
    repo = Path(args.repo).resolve(strict=True)
    source = load_source_authority(Path(args.source_authority), repo)
    plan, plan_bytes = load_json(Path(args.plan), "config authoring plan")
    plan = exact_keys(plan, (
        "bwrap_sha256", "campaign_states", "competition_run_grant_public_key",
        "manifest_directory", "manifest_tool", "prlimit_sha256",
        "run_preflight_grant_public_key", "schema_version", "source_tree_manifests",
        "verifier_bundle_root",
    ), "config authoring plan")
    ensure(plan["schema_version"] == 2, "config authoring requires schema 2")
    bwrap = require_digest(plan["bwrap_sha256"], "bwrap digest")
    prlimit = require_digest(plan["prlimit_sha256"], "prlimit digest")
    require_digest(plan["competition_run_grant_public_key"], "competition public key")
    preflight_key = require_digest(plan["run_preflight_grant_public_key"], "preflight public key")
    registry_root = require_directory(Path(plan["manifest_directory"]), "manifest registry")
    verifier_root = require_directory(Path(plan["verifier_bundle_root"]), "verifier bundle root")
    manifest_tool, manifest_tool_artifact = checked_binding(plan["manifest_tool"], "manifest tool", executable=True)
    ensure(manifest_tool.name == "robin-highscores-manifestctl", "manifest tool filename differs")
    ensure(manifest_tool_artifact["media_type"] == "application/x-executable", "manifest tool media type differs")
    registry = inspect_registry(registry_root, source["source_commit"], preflight_key)
    campaigns = inspect_campaign_states(plan["campaign_states"], registry)
    source_manifests = source_manifest_identities(plan["source_tree_manifests"])
    admission_profiles = profiles(registry, campaigns, source["source_commit"])
    output = Path(args.output)
    ensure(output.is_absolute() and not output.exists(), "config output must be an absent absolute path")
    output_parent = require_directory(output.parent.resolve(strict=True), "config output parent")
    stage = Path(tempfile.mkdtemp(prefix=f".{output.name}.partial-", dir=output_parent))
    try:
        server = stage / "config/highscores-server.toml"
        write_new(server, render_server_config(admission_profiles, source["source_commit"]))
        write_new(stage / "config/api.env", b"RUST_LOG=info\n")
        write_new(stage / "config/worker.env", b"RUST_LOG=info\n")
        catalog_plan = {
            "campaign_state_sources": sorted({record["source"] for record in campaigns.values()}),
            "manifest_directory": str(registry_root),
            "schema_version": 1,
            "source_commit": source["source_commit"],
            "verifier_bundle_root": str(verifier_root),
        }
        catalog_plan_path = stage / "authoring/verifier-catalog-plan-v1.json"
        write_json(catalog_plan_path, catalog_plan)
        catalog_a = stage / "authoring/catalog-a.json"
        catalog_b = stage / "authoring/catalog-b.json"
        command = str(manifest_tool)
        run([command, "author-verifier-catalog-v1", str(catalog_plan_path), str(server), str(catalog_a)])
        run([command, "author-verifier-catalog-v1", str(catalog_plan_path), str(server), str(catalog_b)])
        ensure(catalog_a.read_bytes() == catalog_b.read_bytes(), "double-authored verifier catalogs differ")
        catalog_sha = sha256_file(catalog_a)
        catalog = stage / f"private/verifier/operator-config/{catalog_sha}"
        write_new(catalog, catalog_a.read_bytes())
        catalog_a.unlink()
        catalog_b.unlink()
        run([command, "validate-verifier-catalog-v1", str(catalog_plan_path), str(server), str(catalog)])
        worker = stage / "config/highscores-worker.toml"
        write_new(worker, render_worker_config(source["source_commit"], catalog_sha, registry["verifier_sha256"], source_manifests, bwrap, prlimit))
        with server.open("rb") as stream:
            parsed_server = tomllib.load(stream)
        with worker.open("rb") as stream:
            parsed_worker = tomllib.load(stream)
        release_root = INSTALL_ROOT / "releases" / source["source_commit"]
        ensure(parsed_server["runtime_fence_directory"] == str(STATE_ROOT / "runtime-fence"), "runtime fence path differs")
        ensure(parsed_server["backup_authority_hmac_secret_path"] == str(STATE_ROOT / "api-secrets/backup-authority-hmac.key"), "backup HMAC path differs")
        ensure(parsed_server["backup_manifest_path"] == str(STATE_ROOT / "status/backup-status.json"), "backup status path differs")
        ensure(parsed_server["release_manifest_path"] == str(release_root / "vps-release-manifest-v2.json"), "release manifest path differs")
        ensure(parsed_server["maximum_backup_age_hours"] == 32, "backup maximum age differs")
        ensure(parsed_server["admission_profiles"] == admission_profiles, "server profile TOML roundtrip differs")
        ensure(parsed_worker["verifier_job_config_catalog_sha256"] == catalog_sha, "worker catalog digest differs")
        host_entries = render_host_files(repo, stage, output, source["source_commit"])
        config_entries = []
        for role, name, media in CONFIG_SPECS:
            path = stage / "config" / name
            config_entries.append({"artifact": artifact(path, media), "path": f"config/{name}", "role": role, "source": str(output / "config" / name)})
        fragment = {
            "configs": config_entries,
            "host_files": host_entries,
            "schema_version": 2,
            "source_commit": source["source_commit"],
            "verifier_catalog_plan": str(output / "authoring/verifier-catalog-plan-v1.json"),
            "verifier_operator_config": {"artifact": artifact(catalog, "application/json"), "source": str(output / f"private/verifier/operator-config/{catalog_sha}")},
        }
        write_json(stage / "vps-config-host-fragment-v2.json", fragment)
        write_json(stage / "authoring-evidence-v2.json", {
            "admission_profile_count": len(admission_profiles),
            "build_manifest_sha256": registry["build_digest"],
            "cargo_lock_sha256": source["cargo_lock_sha256"],
            "config_plan_sha256": sha256_bytes(plan_bytes),
            "database_schema_version": source["database_schema_version"],
            "production_authority_public_keys": {
                "competition_run_grant": plan["competition_run_grant_public_key"],
                "run_preflight_grant": preflight_key,
            },
            "schema_version": 2,
            "source_commit": source["source_commit"],
            "source_tree_sha1": source["source_tree_sha1"],
            "verifier_catalog_sha256": catalog_sha,
            "verifier_sha256": registry["verifier_sha256"],
        })
        seal(stage)
        install_directory_no_replace(stage, output)
        stage = None
    finally:
        if stage is not None:
            cleanup_authored_stage(stage)
    print(sha256_file(output / "vps-config-host-fragment-v2.json"))


def read_fragment(path: Path, expected_commit: str) -> dict[str, Any]:
    fragment, _ = load_json(path, "VPS config/host fragment")
    fragment = exact_keys(fragment, ("configs", "host_files", "schema_version", "source_commit", "verifier_catalog_plan", "verifier_operator_config"), "VPS config/host fragment")
    ensure(fragment["schema_version"] == 2 and fragment["source_commit"] == expected_commit, "VPS fragment source/schema differs")
    return fragment


def author_vps_plan(args: argparse.Namespace) -> None:
    repo = Path(args.repo).resolve(strict=True)
    source = load_source_authority(Path(args.source_authority), repo)
    publication = require_directory(Path(args.publication), "PublicationV3")
    config_root = require_directory(Path(args.configs), "config authoring output")
    fragment = read_fragment(config_root / "vps-config-host-fragment-v2.json", source["source_commit"])
    binary_authority_path = Path(args.binary_authority)
    binaries = load_binary_authority(binary_authority_path, source)
    manifest_tool = next(entry["local"] for entry in binaries if entry["role"] == "manifest_tool")
    run([str(manifest_tool), "validate-publication-v3", str(publication)])
    approved_lock = require_digest(args.approved_publication_lock_sha256, "approved PublicationV3 lock")
    lock_sidecar = require_regular(publication / "publication-lock-v3.sha256", "PublicationV3 lock sidecar").read_bytes()
    ensure(lock_sidecar == approved_lock.encode("ascii"), "PublicationV3 lock differs from independent approval")
    remote = Path(args.remote_source_root)
    ensure(remote.is_absolute(), "remote source root must be absolute")
    expected_remote = INSTALL_ROOT / "incoming" / f".sources-{source['source_commit']}"
    ensure(remote == expected_remote, "remote source root must be the exact commit-named incoming .sources path")
    ensure(Path(args.remote_demo_raw_root) == STATE_ROOT / "raw-content/demo", "Demo raw root differs from deployment identity")
    ensure(Path(args.remote_full_raw_root) == STATE_ROOT / "raw-content/full", "Full raw root differs from deployment identity")
    binary_entries = []
    upload_entries = []
    for entry, (role, name, _media) in zip(binaries, BINARY_SPECS, strict=True):
        local = entry["local"]
        authority = entry["artifact"]
        remote_path = remote / "bin" / name
        binary_entries.append({"artifact": authority, "role": role, "source": str(remote_path)})
        upload_entries.append({"artifact": authority, "local": str(local), "remote": str(remote_path)})
    configs = []
    ensure([entry["role"] for entry in fragment["configs"]] == [item[0] for item in CONFIG_SPECS], "config fragment role order differs")
    for entry, (role, name, _media) in zip(fragment["configs"], CONFIG_SPECS, strict=True):
        local = config_root / "config" / name
        checked_artifact(local, entry["artifact"], f"{role} config")
        remote_path = remote / "config" / name
        configs.append({"artifact": entry["artifact"], "role": role, "source": str(remote_path)})
        upload_entries.append({"artifact": entry["artifact"], "local": str(local), "remote": str(remote_path)})
    hosts = []
    ensure([entry["role"] for entry in fragment["host_files"]] == [item[0] for item in HOST_SPECS], "host fragment role order differs")
    for entry, (role, _name, relative, sealed_mode) in zip(fragment["host_files"], HOST_SPECS, strict=True):
        local = config_root / relative
        checked_artifact(local, entry["artifact"], f"{role} host file")
        ensure(stat.S_IMODE(local.stat(follow_symlinks=False).st_mode) == sealed_mode, f"{role} host file mode differs")
        remote_path = remote / "host" / relative
        hosts.append({"artifact": entry["artifact"], "role": role, "source": str(remote_path)})
        upload_entries.append({"artifact": entry["artifact"], "local": str(local), "remote": str(remote_path)})
    output = Path(args.output)
    ensure(output.is_absolute() and not output.exists(), "VPS plan output must be an absent absolute path")
    output_parent = require_directory(output.parent.resolve(strict=True), "VPS plan output parent")
    stage = Path(tempfile.mkdtemp(prefix=f".{output.name}.partial-", dir=output_parent))
    try:
        plan = {
            "binaries": binary_entries,
            "configs": configs,
            "host_files": hosts,
            "private_raw_roots": [
                {"edition": "demo", "root": args.remote_demo_raw_root},
                {"edition": "full", "root": args.remote_full_raw_root},
            ],
            "publication_v3": str(remote / "publication-v3"),
            "schema_version": 2,
            "source_commit": source["source_commit"],
        }
        write_json(stage / "vps-release-plan-v2.json", plan)
        publication_artifact = artifact(publication / "publication-lock-v3.json", "application/json")
        write_json(stage / "release-source-handoff-v2.json", {
            "cargo_lock_sha256": source["cargo_lock_sha256"],
            "binary_authority": artifact(binary_authority_path, "application/json"),
            "database_schema_version": source["database_schema_version"],
            "files": upload_entries,
            "publication": {
                "local": str(publication),
                "publication_lock": publication_artifact,
                "publication_lock_approved_sha256": approved_lock,
                "remote": str(remote / "publication-v3"),
            },
            "remote_source_root": str(remote),
            "schema_version": 2,
            "source_commit": source["source_commit"],
            "source_tree_sha1": source["source_tree_sha1"],
        })
        install_directory_no_replace(stage, output)
        stage = None
    finally:
        if stage is not None:
            cleanup_authored_stage(stage)
    print(sha256_file(output / "vps-release-plan-v2.json"))


def materialize_cloudflare(args: argparse.Namespace) -> None:
    repo = Path(args.repo).resolve(strict=True)
    load_source_authority(Path(args.source_authority), repo)
    expected = require_digest(args.approved_publication_lock_sha256, "approved PublicationV3 lock")
    tool = require_regular(Path(args.manifest_tool), "manifest tool", executable=True)
    ensure(sha256_file(tool) == require_digest(args.manifest_tool_sha256, "approved manifest-tool digest"), "manifest tool differs from independent approval")
    publication = require_directory(Path(args.publication), "PublicationV3")
    output = Path(args.output)
    ensure(output.is_absolute() and not output.exists(), "materialization output must be an absent absolute path")
    print(run([str(tool), "materialize-cloudflare-publication-v3", str(publication), str(output), str(repo), expected]))


def assemble_cloudflare(args: argparse.Namespace) -> None:
    repo = Path(args.repo).resolve(strict=True)
    load_source_authority(Path(args.source_authority), repo)
    receipt = require_digest(args.approved_materialization_receipt_sha256, "approved materialization receipt")
    runtime = require_digest(args.approved_runtime_inventory_sha256, "approved runtime inventory")
    script = require_regular(repo / "wasm-www/scripts/operator-deployment-bundle.mjs", "OperatorBundleV2 script")
    materialization = require_directory(Path(args.materialization), "Cloudflare materialization")
    wasm_static = require_directory(Path(args.wasm_static), "WASM/static handoff")
    output = Path(args.output)
    ensure(output.is_absolute() and not output.exists(), "Cloudflare bundle output must be an absent absolute path")
    node = require_regular(Path(args.node), "Node.js executable", executable=True)
    ensure(sha256_file(node) == require_digest(args.node_sha256, "approved Node.js digest"), "Node.js differs from independent approval")
    print(run([
        str(node),
        str(script),
        "assemble",
        str(materialization),
        str(wasm_static),
        str(output),
        receipt,
        runtime,
        str(repo),
    ]))


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    commands = result.add_subparsers(dest="command", required=True)
    source = commands.add_parser("author-source-authority", help="bind explicit final source/Cargo/database parameters")
    source.add_argument("--repo", required=True)
    source.add_argument("--source-commit", required=True)
    source.add_argument("--source-tree-sha1", required=True)
    source.add_argument("--cargo-lock-sha256", required=True)
    source.add_argument("--database-schema-version", required=True, type=int)
    source.add_argument("--output", required=True)
    source.set_defaults(function=author_source_authority)
    context = commands.add_parser("verify-context", help="verify source commit/tree/Cargo/schema authority")
    context.add_argument("--repo", required=True)
    context.add_argument("--source-authority", required=True)
    context.set_defaults(function=lambda args: print(canonical_bytes(load_source_authority(Path(args.source_authority), Path(args.repo).resolve(strict=True))).decode()))
    configs = commands.add_parser("author-configs", help="author production configs and verifier catalog")
    configs.add_argument("--repo", required=True)
    configs.add_argument("--source-authority", required=True)
    configs.add_argument("--plan", required=True)
    configs.add_argument("--output", required=True)
    configs.set_defaults(function=author_configs)
    vps = commands.add_parser("author-vps-plan", help="author VpsReleasePlanV2 and upload map")
    vps.add_argument("--repo", required=True)
    vps.add_argument("--source-authority", required=True)
    vps.add_argument("--publication", required=True)
    vps.add_argument("--binary-authority", required=True)
    vps.add_argument("--configs", required=True)
    vps.add_argument("--approved-publication-lock-sha256", required=True)
    vps.add_argument("--remote-source-root", required=True)
    vps.add_argument("--remote-demo-raw-root", required=True)
    vps.add_argument("--remote-full-raw-root", required=True)
    vps.add_argument("--output", required=True)
    vps.set_defaults(function=author_vps_plan)
    materialize = commands.add_parser("materialize-cloudflare", help="materialize an approved PublicationV3")
    materialize.add_argument("--repo", required=True)
    materialize.add_argument("--source-authority", required=True)
    materialize.add_argument("--manifest-tool", required=True)
    materialize.add_argument("--manifest-tool-sha256", required=True)
    materialize.add_argument("--publication", required=True)
    materialize.add_argument("--approved-publication-lock-sha256", required=True)
    materialize.add_argument("--output", required=True)
    materialize.set_defaults(function=materialize_cloudflare)
    cloudflare = commands.add_parser("assemble-cloudflare", help="assemble accepted six-argument OperatorBundleV2")
    cloudflare.add_argument("--repo", required=True)
    cloudflare.add_argument("--source-authority", required=True)
    cloudflare.add_argument("--node", required=True)
    cloudflare.add_argument("--node-sha256", required=True)
    cloudflare.add_argument("--materialization", required=True)
    cloudflare.add_argument("--wasm-static", required=True)
    cloudflare.add_argument("--approved-materialization-receipt-sha256", required=True)
    cloudflare.add_argument("--approved-runtime-inventory-sha256", required=True)
    cloudflare.add_argument("--output", required=True)
    cloudflare.set_defaults(function=assemble_cloudflare)
    return result


def main() -> int:
    arguments = parser().parse_args()
    try:
        arguments.function(arguments)
    except (AuthoringError, OSError, subprocess.SubprocessError, KeyError, TypeError, ValueError) as error:
        print(f"release authoring failed closed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
