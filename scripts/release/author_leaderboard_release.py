#!/usr/bin/env python3
"""Author leaderboard server/worker configs and the verifier catalog.

This is only needed when a new ranked authority release (verifier binary,
manifest registry, campaign states) is deliberately published. Routine service
deploys reuse the live configs. The tool writes local files only; it has no
SSH, Cloudflare, or service-manager authority.
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
INSTALL_ROOT = Path("/home/robinhood/.local/opt/robin-highscores")
STATE_ROOT = Path("/home/robinhood/.local/share/robin-highscores")
CONFIG_ROOT = Path("/home/robinhood/.config/robin-highscores")
CAMPAIGN_MEDIA_TYPE = "application/x-robin-campaign+bitcode"

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
CONFIG_FILES = ("server.toml", "worker.toml", "api.env", "worker.env")


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


def require_regular(path: Path, label: str, *, executable: bool = False) -> Path:
    try:
        metadata = path.lstat()
    except FileNotFoundError as error:
        raise AuthoringError(f"missing {label}: {path}") from error
    ensure(path.is_absolute(), f"{label} is not an absolute path: {path}")
    ensure(stat.S_ISREG(metadata.st_mode), f"{label} is not a regular file: {path}")
    if executable:
        ensure(metadata.st_mode & 0o111 != 0, f"{label} is not executable: {path}")
    return path


def require_directory(path: Path, label: str) -> Path:
    ensure(path.is_absolute(), f"{label} is not an absolute path: {path}")
    ensure(path.is_dir() and not path.is_symlink(), f"{label} is not a real directory: {path}")
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
    with open(path, "xb") as stream:
        stream.write(data)
    path.chmod(mode)


def write_json(path: Path, value: Any, mode: int = 0o600) -> None:
    write_new(path, canonical_bytes(value), mode)


def install_directory_no_replace(stage: Path, output: Path) -> None:
    """Move a finished staging tree into place, refusing to replace output."""
    ensure(not os.path.lexists(output), f"output already exists: {output}")
    # os.rename onto an existing empty directory would succeed on Linux, so
    # the check above is required; the window between them is acceptable for a
    # single local operator.
    os.rename(stage, output)


def run(arguments: list[str], *, cwd: Path | None = None) -> str:
    completed = subprocess.run(arguments, cwd=cwd, text=True, capture_output=True)
    if completed.returncode != 0:
        raise AuthoringError(
            f"command failed ({completed.returncode}): {' '.join(arguments)}\n"
            f"stdout:\n{completed.stdout}\nstderr:\n{completed.stderr}"
        )
    return completed.stdout.strip()


def addressed_documents(root: Path, name: str, *, published: bool = False) -> dict[str, dict[str, Any]]:
    directory = require_directory(root / name, f"{name} registry")
    result: dict[str, dict[str, Any]] = {}
    for path in sorted(directory.iterdir(), key=lambda item: item.name):
        ensure(path.suffix == ".json", f"unexpected non-JSON {name} entry: {path.name}")
        identity = require_digest(path.stem, f"{name} filename")
        document, data = load_json(path, f"{name} document")
        ensure(isinstance(document, dict), f"{name} document is not an object")
        if published:
            ensure(document.get("ruleset_manifest_sha256") == identity, "published-rulesets filename identity differs")
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
        checked_artifact(Path(record["source"]), record["artifact"], "campaign state")
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
            allowed_scopes = ["individual_level"] if edition == "demo" else (["individual_level", "campaign_continuation"] if kind == "field_mission" else ["campaign_continuation"])
            if edition == "full" and kind == "field_mission" and mission == "H01_Lin_VL":
                allowed_scopes = ["individual_level", "campaign_genesis", "campaign_continuation"]
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
    # `commit` is the authority release: manifests and campaign states live in
    # its private tree, which routine service deploys never replace.
    release_root = INSTALL_ROOT / "releases" / commit
    fields = (
        ("bind", "127.0.0.1:8787"),
        ("database_path", str(STATE_ROOT / "database/highscores.sqlite3")),
        ("replay_directory", str(STATE_ROOT / "replays")),
        ("campaign_state_directory", str(STATE_ROOT / "campaign-states")),
        ("cursor_secret_path", str(STATE_ROOT / "api-secrets/cursor-hmac.key")),
        ("competition_run_grant_secret_path", str(STATE_ROOT / "api-secrets/competition-run-grant.key")),
        ("run_preflight_grant_secret_path", str(STATE_ROOT / "api-secrets/run-preflight-grant.key")),
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
    )
    lines = ["# Authored production leaderboard configuration."]
    lines.extend(f"{key} = {toml_value(value)}" for key, value in fields)
    for profile in admission_profiles:
        lines.extend(("", "[[admission_profiles]]"))
        lines.extend(f"{key} = {toml_value(value)}" for key, value in profile.items())
    return ("\n".join(lines) + "\n").encode()


def render_worker_config(commit: str, catalog: str, verifier: str, source_manifests: dict[str, str], bwrap: str, prlimit: str) -> bytes:
    release_root = INSTALL_ROOT / "releases" / commit
    lines = [
        "# Authored production verifier-worker configuration.",
        f'server_config = "{CONFIG_ROOT}/server.toml"',
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


def author_configs(args: argparse.Namespace) -> None:
    commit = require_commit(args.source_commit)
    plan, plan_bytes = load_json(Path(args.plan), "config authoring plan")
    plan = exact_keys(plan, (
        "bwrap_sha256", "campaign_states", "competition_run_grant_public_key",
        "manifest_directory", "manifest_tool", "prlimit_sha256",
        "run_preflight_grant_public_key", "schema_version", "source_tree_manifests",
        "verifier_bundle_root",
    ), "config authoring plan")
    ensure(plan["schema_version"] == 3, "config authoring requires plan schema 3")
    bwrap = require_digest(plan["bwrap_sha256"], "bwrap digest")
    prlimit = require_digest(plan["prlimit_sha256"], "prlimit digest")
    require_digest(plan["competition_run_grant_public_key"], "competition public key")
    preflight_key = require_digest(plan["run_preflight_grant_public_key"], "preflight public key")
    registry_root = require_directory(Path(plan["manifest_directory"]), "manifest registry")
    verifier_root = require_directory(Path(plan["verifier_bundle_root"]), "verifier bundle root")
    ensure(isinstance(plan["manifest_tool"], str), "manifest_tool must be a path string")
    manifest_tool = require_regular(Path(plan["manifest_tool"]), "manifest tool", executable=True)
    registry = inspect_registry(registry_root, commit, preflight_key)
    campaigns = inspect_campaign_states(plan["campaign_states"], registry)
    source_manifests = source_manifest_identities(plan["source_tree_manifests"])
    admission_profiles = profiles(registry, campaigns, commit)
    output = Path(args.output)
    ensure(output.is_absolute() and not output.exists(), "config output must be an absent absolute path")
    output_parent = require_directory(output.parent.resolve(strict=True), "config output parent")
    stage: Path | None = Path(tempfile.mkdtemp(prefix=f".{output.name}.partial-", dir=output_parent))
    try:
        server = stage / "config/server.toml"
        write_new(server, render_server_config(admission_profiles, commit))
        write_new(stage / "config/api.env", b"RUST_LOG=info\n")
        write_new(stage / "config/worker.env", b"RUST_LOG=info\n")
        catalog_plan = {
            "campaign_state_sources": sorted({record["source"] for record in campaigns.values()}),
            "manifest_directory": str(registry_root),
            "schema_version": 1,
            "source_commit": commit,
            "verifier_bundle_root": str(verifier_root),
        }
        catalog_plan_path = stage / "authoring/verifier-catalog-plan-v1.json"
        write_json(catalog_plan_path, catalog_plan)
        authored_catalog = stage / "authoring/catalog.json"
        command = str(manifest_tool)
        run([command, "author-verifier-catalog-v1", str(catalog_plan_path), str(server), str(authored_catalog)])
        catalog_sha = sha256_file(authored_catalog)
        catalog = stage / f"private/verifier/operator-config/{catalog_sha}"
        catalog.parent.mkdir(parents=True)
        authored_catalog.rename(catalog)
        run([command, "validate-verifier-catalog-v1", str(catalog_plan_path), str(server), str(catalog)])
        worker = stage / "config/worker.toml"
        write_new(worker, render_worker_config(commit, catalog_sha, registry["verifier_sha256"], source_manifests, bwrap, prlimit))
        with server.open("rb") as stream:
            parsed_server = tomllib.load(stream)
        with worker.open("rb") as stream:
            parsed_worker = tomllib.load(stream)
        ensure(parsed_server["admission_profiles"] == admission_profiles, "server profile TOML roundtrip differs")
        ensure(parsed_worker["verifier_job_config_catalog_sha256"] == catalog_sha, "worker catalog digest differs")
        write_json(stage / "authoring-evidence.json", {
            "admission_profile_count": len(admission_profiles),
            "build_manifest_sha256": registry["build_digest"],
            "config_plan_sha256": sha256_bytes(plan_bytes),
            "config_sha256": {name: sha256_file(stage / "config" / name) for name in CONFIG_FILES},
            "production_authority_public_keys": {
                "competition_run_grant": plan["competition_run_grant_public_key"],
                "run_preflight_grant": preflight_key,
            },
            "schema_version": 3,
            "source_commit": commit,
            "verifier_catalog_sha256": catalog_sha,
            "verifier_sha256": registry["verifier_sha256"],
        })
        install_directory_no_replace(stage, output)
        stage = None
    finally:
        if stage is not None:
            shutil.rmtree(stage)
    print(sha256_file(output / "authoring-evidence.json"))


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    commands = result.add_subparsers(dest="command", required=True)
    configs = commands.add_parser("author-configs", help="author production configs and verifier catalog")
    configs.add_argument(
        "--source-commit",
        required=True,
        help="authority release commit; must equal the BuildManifestV2 source_commit",
    )
    configs.add_argument("--plan", required=True)
    configs.add_argument("--output", required=True)
    configs.set_defaults(function=author_configs)
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
