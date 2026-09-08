#!/usr/bin/env python3

from __future__ import annotations

import argparse
import contextlib
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import tomllib
import unittest
from unittest import mock
import io


MODULE_PATH = Path(__file__).with_name("author_leaderboard_release.py")
SPEC = importlib.util.spec_from_file_location("author_leaderboard_release", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)


def canonical(value: object) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


class ReleaseAuthoringTests(unittest.TestCase):
    def test_real_runtime_fence_gate_host_authority_is_frozen_and_executable(self) -> None:
        gate_specs = release.HOST_SPECS[8:11]
        self.assertEqual(
            gate_specs,
            (
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
            ),
        )
        self.assertEqual(len(release.HOST_SPECS), 19)

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            repo = root / "repo"
            source_root = repo / "crates/robin_highscores/deploy"
            for index, (_role, source, _target, _mode) in enumerate(release.HOST_SPECS):
                path = source_root / source
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(f"fixture {index}\n", encoding="utf-8")
            subprocess.run(["git", "init", "-q", str(repo)], check=True)
            subprocess.run(["git", "-C", str(repo), "config", "user.email", "test@example.invalid"], check=True)
            subprocess.run(["git", "-C", str(repo), "config", "user.name", "test"], check=True)
            subprocess.run(["git", "-C", str(repo), "add", "."], check=True)
            subprocess.run(["git", "-C", str(repo), "commit", "-qm", "fixture"], check=True)
            commit = subprocess.check_output(
                ["git", "-C", str(repo), "rev-parse", "HEAD"], text=True
            ).strip()
            stage = root / "stage"
            final = root / "final"
            stage.mkdir()
            rendered = release.render_host_files(repo, stage, final, commit)
            self.assertEqual(
                [entry["role"] for entry in rendered],
                [spec[0] for spec in release.HOST_SPECS],
            )
            for role, _source, target, sealed_mode in gate_specs:
                path = stage / target
                self.assertEqual(os.stat(path, follow_symlinks=False).st_mode & 0o777, 0o750)
                entry = next(item for item in rendered if item["role"] == role)
                expected_media = (
                    "application/x-sh"
                    if target.endswith(".sh")
                    else "text/x-python; charset=utf-8"
                )
                self.assertEqual(entry["artifact"]["media_type"], expected_media)
                self.assertEqual(entry["source"], str(final / target))
            guide = repo / "crates/robin_highscores/README.md"
            self.assertEqual((stage / "deploy/README.md").read_bytes(), guide.read_bytes())
            self.assertEqual(
                (stage / "deploy/VPS_RELEASE_INSTALL.md").read_text(),
                "See the [combined guide](README.md#vps-installation-and-rollback).\n",
            )
            self.assertEqual(
                (stage / "deploy/BACKUP_RESTORE.md").read_text(),
                "See the [combined guide](README.md#backup-and-disaster-recovery).\n",
            )
            guide.write_text("unreviewed guide\n", encoding="utf-8")
            with self.assertRaisesRegex(release.AuthoringError, "deployment source differs"):
                release.render_host_files(repo, root / "tampered-stage", final, commit)
            release.seal(stage)
            for _role, _source, target, sealed_mode in gate_specs:
                self.assertEqual(
                    os.stat(stage / target, follow_symlinks=False).st_mode & 0o777,
                    sealed_mode,
                )
            release.cleanup_authored_stage(stage)

    def test_vps_plan_emits_every_gate_role_and_rejects_gate_mode_drift(self) -> None:
        source = {
            "cargo_lock_sha256": "1" * 64,
            "database_schema_version": 2,
            "schema_version": 1,
            "source_commit": "2" * 40,
            "source_tree_sha1": "3" * 40,
        }
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            repo = root / "repo"
            repo.mkdir()
            publication = root / "publication"
            publication.mkdir()
            approved_lock = "4" * 64
            (publication / "publication-lock-v3.sha256").write_text(
                approved_lock, encoding="ascii"
            )
            (publication / "publication-lock-v3.json").write_bytes(b"{}")
            configs = root / "configs"
            configs.mkdir()

            config_entries = []
            for role, name, media_type in release.CONFIG_SPECS:
                path = configs / "config" / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(f"{role}\n", encoding="utf-8")
                path.chmod(0o440)
                config_entries.append(
                    {
                        "artifact": release.artifact(path, media_type),
                        "path": f"config/{name}",
                        "role": role,
                        "source": str(path),
                    }
                )

            host_entries = []
            for role, _source, relative, sealed_mode in release.HOST_SPECS:
                path = configs / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(f"{role}\n", encoding="utf-8")
                path.chmod(sealed_mode)
                media_type = (
                    "application/x-sh"
                    if relative.endswith(".sh")
                    else "text/x-python; charset=utf-8"
                    if relative.endswith(".py")
                    else "text/plain; charset=utf-8"
                )
                host_entries.append(
                    {
                        "artifact": release.artifact(path, media_type),
                        "path": relative,
                        "role": role,
                        "source": str(path),
                    }
                )
            fragment = {
                "configs": config_entries,
                "host_files": host_entries,
                "schema_version": 2,
                "source_commit": source["source_commit"],
                "verifier_catalog_plan": str(configs / "catalog-plan.json"),
                "verifier_operator_config": {
                    "artifact": {"byte_length": 1, "media_type": "application/json", "sha256": "5" * 64},
                    "source": str(configs / "catalog.json"),
                },
            }
            (configs / "vps-config-host-fragment-v2.json").write_bytes(canonical(fragment))

            binaries = []
            for index, (role, name, media_type) in enumerate(release.BINARY_SPECS):
                path = root / "bin" / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(f"binary {index}\n", encoding="utf-8")
                path.chmod(0o550)
                binaries.append(
                    {"artifact": release.artifact(path, media_type), "local": path, "role": role}
                )
            binary_authority = root / "musl-binary-authority-v2.json"
            binary_authority.write_bytes(b"{}")
            remote = (
                release.INSTALL_ROOT
                / "incoming"
                / f".sources-{source['source_commit']}"
            )
            arguments = argparse.Namespace(
                approved_publication_lock_sha256=approved_lock,
                binary_authority=str(binary_authority),
                configs=str(configs),
                output=str(root / "plan"),
                publication=str(publication),
                remote_demo_raw_root=str(release.STATE_ROOT / "raw-content/demo"),
                remote_full_raw_root=str(release.STATE_ROOT / "raw-content/full"),
                remote_source_root=str(remote),
                repo=str(repo),
                source_authority=str(root / "source.json"),
            )
            with mock.patch.object(release, "load_source_authority", return_value=source), mock.patch.object(
                release, "load_binary_authority", return_value=binaries
            ), mock.patch.object(release, "run", return_value="validated"):
                with contextlib.redirect_stdout(io.StringIO()):
                    release.author_vps_plan(arguments)
                plan = json.loads((root / "plan/vps-release-plan-v2.json").read_bytes())
                self.assertEqual(
                    [entry["role"] for entry in plan["host_files"]],
                    [spec[0] for spec in release.HOST_SPECS],
                )
                self.assertEqual(
                    [entry["role"] for entry in plan["host_files"]][8:11],
                    [spec[0] for spec in release.HOST_SPECS[8:11]],
                )

                gate = configs / release.HOST_SPECS[9][2]
                gate.chmod(0o440)
                arguments.output = str(root / "rejected-plan")
                with self.assertRaisesRegex(release.AuthoringError, "host file mode differs"):
                    release.author_vps_plan(arguments)

    def test_server_config_uses_v2_release_and_authenticated_backup_paths(self) -> None:
        commit = "a" * 40
        parsed = tomllib.loads(release.render_server_config([], commit).decode())
        state = "/home/robinhood/.local/share/robin-highscores"
        install = "/home/robinhood/.local/opt/robin-highscores"
        self.assertEqual(parsed["runtime_fence_directory"], f"{state}/runtime-fence")
        self.assertEqual(
            parsed["backup_authority_hmac_secret_path"],
            f"{state}/api-secrets/backup-authority-hmac.key",
        )
        self.assertEqual(parsed["backup_manifest_path"], f"{state}/status/backup-status.json")
        self.assertEqual(
            parsed["release_manifest_path"],
            f"{install}/releases/{commit}/vps-release-manifest-v2.json",
        )
        self.assertEqual(parsed["maximum_backup_age_hours"], 32)

    def test_worker_config_is_exact_commit_named_and_has_runtime_digests(self) -> None:
        commit = "b" * 40
        catalog = "c" * 64
        verifier = "d" * 64
        demo = "e" * 64
        full = "f" * 64
        bwrap = "1" * 64
        prlimit = "2" * 64
        parsed = tomllib.loads(
            release.render_worker_config(
                commit,
                catalog,
                verifier,
                {"demo": demo, "full": full},
                bwrap,
                prlimit,
            ).decode()
        )
        prefix = f"/home/robinhood/.local/opt/robin-highscores/releases/{commit}"
        self.assertEqual(parsed["server_config"], f"{prefix}/config/highscores-server.toml")
        self.assertEqual(parsed["demo_raw_content_manifest"], f"{prefix}/private/source-tree-manifests-v2/{demo}.json")
        self.assertEqual(parsed["full_raw_content_manifest"], f"{prefix}/private/source-tree-manifests-v2/{full}.json")
        self.assertEqual(parsed["verifier_launcher"]["bwrap_sha256"], bwrap)
        self.assertEqual(parsed["verifier_launcher"]["prlimit_sha256"], prlimit)
        self.assertEqual(parsed["verifier_launcher"]["verifier_sha256"], verifier)

    def test_source_authority_is_runtime_data_and_rejects_schema_drift(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            (root / "crates/robin_run_protocol/src").mkdir(parents=True)
            (root / "Cargo.lock").write_bytes(b"runtime lock\n")
            schema_path = root / "crates/robin_run_protocol/src/lib.rs"
            schema_path.write_text(
                "pub const HIGHSCORES_DATABASE_SCHEMA_VERSION: i64 = 7;\n",
                encoding="utf-8",
            )
            subprocess.run(["git", "init", "-q", str(root)], check=True)
            subprocess.run(["git", "-C", str(root), "config", "user.email", "test@example.invalid"], check=True)
            subprocess.run(["git", "-C", str(root), "config", "user.name", "test"], check=True)
            subprocess.run(["git", "-C", str(root), "add", "."], check=True)
            subprocess.run(["git", "-C", str(root), "commit", "-qm", "fixture"], check=True)
            commit = subprocess.check_output(["git", "-C", str(root), "rev-parse", "HEAD"], text=True).strip()
            tree = subprocess.check_output(["git", "-C", str(root), "rev-parse", "HEAD^{tree}"], text=True).strip()
            authority = {
                "cargo_lock_sha256": hashlib.sha256(b"runtime lock\n").hexdigest(),
                "database_schema_version": 7,
                "schema_version": 1,
                "source_commit": commit,
                "source_tree_sha1": tree,
            }
            authority_path = root.parent / f"{root.name}-source.json"
            authority_path.write_bytes(canonical(authority))
            try:
                self.assertEqual(release.load_source_authority(authority_path, root), authority)
                authority["database_schema_version"] = 8
                authority_path.unlink()
                authority_path.write_bytes(canonical(authority))
                with self.assertRaisesRegex(release.AuthoringError, "database schema differs"):
                    release.load_source_authority(authority_path, root)
            finally:
                authority_path.unlink(missing_ok=True)

    def test_musl_binary_authority_rehashes_every_runtime_binding(self) -> None:
        source = {
            "cargo_lock_sha256": "1" * 64,
            "database_schema_version": 2,
            "schema_version": 1,
            "source_commit": "2" * 40,
            "source_tree_sha1": "3" * 40,
        }
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            entries = []
            for index, (role, name, media) in enumerate(release.BINARY_SPECS, start=1):
                path = root / name
                data = f"binary {index}".encode()
                path.write_bytes(data)
                path.chmod(0o700)
                entries.append(
                    {
                        "artifact": {
                            "byte_length": len(data),
                            "media_type": media,
                            "sha256": hashlib.sha256(data).hexdigest(),
                        },
                        "role": role,
                        "source": str(path),
                    }
                )
            authority = {
                "binaries": entries,
                "cargo_lock_sha256": source["cargo_lock_sha256"],
                "schema_version": 2,
                "source_commit": source["source_commit"],
                "source_tree_sha1": source["source_tree_sha1"],
            }
            path = root / "musl-binary-authority-v2.json"
            path.write_bytes(canonical(authority))
            self.assertEqual(len(release.load_binary_authority(path, source)), 5)
            entries[0]["artifact"]["sha256"] = "4" * 64
            path.unlink()
            path.write_bytes(canonical(authority))
            with self.assertRaisesRegex(release.AuthoringError, "differs from its artifact authority"):
                release.load_binary_authority(path, source)

    def test_source_authority_rejects_git_replacement_objects(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            (root / "crates/robin_run_protocol/src").mkdir(parents=True)
            cargo = root / "Cargo.lock"
            schema = root / "crates/robin_run_protocol/src/lib.rs"
            cargo.write_bytes(b"original lock\n")
            schema.write_text(
                "pub const HIGHSCORES_DATABASE_SCHEMA_VERSION: i64 = 2;\n",
                encoding="utf-8",
            )
            subprocess.run(["git", "init", "-q", str(root)], check=True)
            subprocess.run(["git", "-C", str(root), "config", "user.email", "test@example.invalid"], check=True)
            subprocess.run(["git", "-C", str(root), "config", "user.name", "test"], check=True)
            subprocess.run(["git", "-C", str(root), "add", "."], check=True)
            subprocess.run(["git", "-C", str(root), "commit", "-qm", "original"], check=True)
            original = subprocess.check_output(["git", "-C", str(root), "rev-parse", "HEAD"], text=True).strip()
            original_tree = subprocess.check_output(["git", "-C", str(root), "rev-parse", "HEAD^{tree}"], text=True).strip()
            cargo.write_bytes(b"replacement lock\n")
            schema.write_text(
                "pub const HIGHSCORES_DATABASE_SCHEMA_VERSION: i64 = 99;\n",
                encoding="utf-8",
            )
            subprocess.run(["git", "-C", str(root), "add", "."], check=True)
            subprocess.run(["git", "-C", str(root), "commit", "-qm", "replacement"], check=True)
            replacement = subprocess.check_output(["git", "-C", str(root), "rev-parse", "HEAD"], text=True).strip()
            subprocess.run(["git", "-C", str(root), "reset", "--hard", "-q", original], check=True)
            subprocess.run(["git", "-C", str(root), "replace", original, replacement], check=True)
            authority = {
                "cargo_lock_sha256": hashlib.sha256(b"original lock\n").hexdigest(),
                "database_schema_version": 2,
                "schema_version": 1,
                "source_commit": original,
                "source_tree_sha1": original_tree,
            }
            authority_path = root.parent / f"{root.name}-source.json"
            authority_path.write_bytes(canonical(authority))
            try:
                self.assertNotEqual(
                    subprocess.check_output(["git", "-C", str(root), "rev-parse", "HEAD^{tree}"], text=True).strip(),
                    original_tree,
                    "fixture must demonstrate replacement-tree substitution",
                )
                with self.assertRaisesRegex(release.AuthoringError, "forbidden replacement refs"):
                    release.load_source_authority(authority_path, root)
            finally:
                authority_path.unlink(missing_ok=True)

    def test_cloudflare_bundle_uses_accepted_six_argument_contract(self) -> None:
        source = {
            "cargo_lock_sha256": "1" * 64,
            "database_schema_version": 2,
            "schema_version": 1,
            "source_commit": "2" * 40,
            "source_tree_sha1": "3" * 40,
        }
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            repo = root / "repo"
            materialization = root / "materialization"
            wasm_static = root / "wasm-static"
            output = root / "bundle"
            node = root / "node"
            script = repo / "wasm-www/scripts/operator-deployment-bundle.mjs"
            script.parent.mkdir(parents=True)
            materialization.mkdir()
            wasm_static.mkdir()
            script.write_text("// fixture\n", encoding="utf-8")
            node.write_bytes(b"approved node")
            node.chmod(0o700)
            source_path = root / "source.json"
            source_path.write_bytes(canonical(source))
            arguments = argparse.Namespace(
                approved_materialization_receipt_sha256="4" * 64,
                approved_runtime_inventory_sha256="5" * 64,
                materialization=str(materialization),
                node=str(node),
                node_sha256=hashlib.sha256(b"approved node").hexdigest(),
                output=str(output),
                repo=str(repo),
                source_authority=str(source_path),
                wasm_static=str(wasm_static),
            )
            with mock.patch.object(release, "load_source_authority", return_value=source), mock.patch.object(
                release, "run", return_value="receipt"
            ) as execute:
                with contextlib.redirect_stdout(io.StringIO()):
                    release.assemble_cloudflare(arguments)
            invoked = execute.call_args.args[0]
            self.assertEqual(
                invoked,
                [
                    str(node),
                    str(script),
                    "assemble",
                    str(materialization),
                    str(wasm_static),
                    str(output),
                    "4" * 64,
                    "5" * 64,
                    str(repo),
                ],
            )
            self.assertEqual(len(invoked[3:]), 6)

    def test_atomic_directory_install_never_replaces_existing_output(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            stage = root / "stage"
            output = root / "output"
            stage.mkdir()
            (stage / "new").write_text("new", encoding="utf-8")
            output.mkdir()
            (output / "old").write_text("old", encoding="utf-8")
            with self.assertRaisesRegex(release.AuthoringError, "appeared concurrently"):
                release.install_directory_no_replace(stage, output)
            self.assertEqual((output / "old").read_text(encoding="utf-8"), "old")
            self.assertTrue((stage / "new").is_file())

            fresh = root / "fresh"
            release.install_directory_no_replace(stage, fresh)
            self.assertFalse(stage.exists())
            self.assertEqual((fresh / "new").read_text(encoding="utf-8"), "new")

            sealed_stage = root / "sealed-stage"
            (sealed_stage / "nested").mkdir(parents=True)
            (sealed_stage / "nested/artifact").write_text("sealed", encoding="utf-8")
            release.seal(sealed_stage)
            with self.assertRaisesRegex(release.AuthoringError, "appeared concurrently"):
                release.install_directory_no_replace(sealed_stage, output)
            release.cleanup_authored_stage(sealed_stage)
            self.assertFalse(sealed_stage.exists(), "failed no-replace install retained a sealed staging tree")

    def test_authoring_source_has_no_retired_release_names_or_fixed_source(self) -> None:
        text = MODULE_PATH.read_text(encoding="utf-8")
        self.assertNotIn("publication-v2", text.lower())
        self.assertNotIn("vps-release-plan-v1", text.lower())
        self.assertNotRegex(text, r"[0-9a-f]{40}.*d4e6")


if __name__ == "__main__":
    unittest.main()
