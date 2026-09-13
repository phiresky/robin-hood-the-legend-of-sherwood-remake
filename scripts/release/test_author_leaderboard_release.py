#!/usr/bin/env python3

from __future__ import annotations

import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import tomllib
import unittest


MODULE_PATH = Path(__file__).with_name("author_leaderboard_release.py")
SPEC = importlib.util.spec_from_file_location("author_leaderboard_release", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)


class ReleaseAuthoringTests(unittest.TestCase):
    def test_server_config_has_no_retired_deploy_fields(self) -> None:
        commit = "a" * 40
        parsed = tomllib.loads(release.render_server_config([], commit).decode())
        for retired in (
            "runtime_fence_directory",
            "backup_authority_hmac_secret_path",
            "backup_manifest_path",
            "release_manifest_path",
            "maximum_backup_age_hours",
        ):
            self.assertNotIn(retired, parsed)
        self.assertEqual(
            parsed["manifest_directory"],
            f"/home/robinhood/.local/opt/robin-highscores/releases/{commit}/config/manifests",
        )
        self.assertEqual(
            parsed["database_path"],
            "/home/robinhood/.local/share/robin-highscores/database/highscores.sqlite3",
        )

    def test_worker_config_points_at_authority_release_and_live_server_config(self) -> None:
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
        self.assertEqual(parsed["server_config"], "/home/robinhood/.config/robin-highscores/server.toml")
        self.assertEqual(parsed["verifier_job_config_catalog"], f"{prefix}/private/verifier/operator-config/{catalog}")
        self.assertEqual(parsed["demo_raw_content_manifest"], f"{prefix}/private/source-tree-manifests-v2/{demo}.json")
        self.assertEqual(parsed["full_raw_content_manifest"], f"{prefix}/private/source-tree-manifests-v2/{full}.json")
        self.assertEqual(parsed["verifier_launcher"]["verifier_program"], f"{prefix}/bin/robin-replay-verifier")
        self.assertEqual(parsed["verifier_launcher"]["bwrap_sha256"], bwrap)
        self.assertEqual(parsed["verifier_launcher"]["prlimit_sha256"], prlimit)
        self.assertEqual(parsed["verifier_launcher"]["verifier_sha256"], verifier)

    def test_campaign_states_are_rehashed_against_their_artifacts(self) -> None:
        config = "3" * 64
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary).resolve() / "state"
            source.write_bytes(b"campaign")
            record = {
                "artifact": {
                    "byte_length": len(b"campaign"),
                    "media_type": release.CAMPAIGN_MEDIA_TYPE,
                    "sha256": hashlib.sha256(b"campaign").hexdigest(),
                },
                "edition": "demo",
                "kind": "individual_template",
                "rules_config_sha256": config,
                "source": str(source),
            }
            registry = {"rulesets": {("demo", config): ("4" * 64, {})}}
            states = release.inspect_campaign_states([record], registry)
            self.assertEqual(states[("demo", config)], record)
            source.write_bytes(b"tampered")
            with self.assertRaisesRegex(release.AuthoringError, "differs from its artifact authority"):
                release.inspect_campaign_states([record], registry)

    def test_directory_install_never_replaces_existing_output(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            stage = root / "stage"
            output = root / "output"
            stage.mkdir()
            (stage / "new").write_text("new", encoding="utf-8")
            output.mkdir()
            (output / "old").write_text("old", encoding="utf-8")
            with self.assertRaisesRegex(release.AuthoringError, "already exists"):
                release.install_directory_no_replace(stage, output)
            self.assertEqual((output / "old").read_text(encoding="utf-8"), "old")
            self.assertTrue((stage / "new").is_file())

            fresh = root / "fresh"
            release.install_directory_no_replace(stage, fresh)
            self.assertFalse(stage.exists())
            self.assertEqual((fresh / "new").read_text(encoding="utf-8"), "new")

    def test_only_config_authoring_command_remains(self) -> None:
        commands = release.parser()._subparsers._group_actions[0].choices
        self.assertEqual(sorted(commands), ["author-configs"])
        text = MODULE_PATH.read_text(encoding="utf-8")
        for retired in ("runtime-fence", "publication-v3", "vps-release", "operator-deployment-bundle"):
            self.assertNotIn(retired, text.lower())


if __name__ == "__main__":
    unittest.main()
