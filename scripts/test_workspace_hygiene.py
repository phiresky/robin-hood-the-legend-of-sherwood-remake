#!/usr/bin/env python3
"""Read-only repository checks and an isolated agent-launcher fixture."""

from pathlib import Path
import os
import subprocess
import tempfile
import tomllib
import unittest


ROOT = Path(__file__).resolve().parents[1]


class WorkspaceHygieneTests(unittest.TestCase):
    def test_tracked_mod_examples_allow_new_files_without_forcing_git_add(self):
        paths = [
            "mods/multi-team-demos/new-example.json",
            "mods/timed-ambience-demo/new-example.json",
            ".gitmodules",
        ]
        result = subprocess.run(
            ["git", "check-ignore", "--no-index", "--stdin"],
            cwd=ROOT,
            input="\n".join(paths) + "\n",
            text=True,
            capture_output=True,
        )
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertEqual(result.stdout, "")

    def test_local_outputs_stay_ignored(self):
        paths = ["mods/local-install/new.json", ".agents/session.json", "tmp/example.log"]
        result = subprocess.run(
            ["git", "check-ignore", "--no-index", "--stdin"],
            cwd=ROOT,
            input="\n".join(paths) + "\n",
            text=True,
            capture_output=True,
            check=True,
        )
        self.assertEqual(result.stdout.splitlines(), paths)

    def test_agent_launcher_uses_matching_branch_and_portable_config_paths(self):
        with tempfile.TemporaryDirectory(prefix="robin-launcher-test-") as scratch:
            root = Path(scratch)
            subprocess.run(["git", "init", "-b", "main", str(root)], check=True, capture_output=True)
            subprocess.run(
                ["git", "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                 "commit", "--allow-empty", "-m", "fixture"],
                cwd=root, check=True, capture_output=True,
            )
            tools = root / "tools"
            tools.mkdir()
            tmux = tools / "tmux"
            tmux.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
            tmux.chmod(0o755)
            env = dict(os.environ)
            env.update({
                "PATH": f"{tools}{os.pathsep}{env['PATH']}",
                "AGENT": "codex",
                "CARGO_HOME": str(root / 'cargo "fixture"'),
                "XDG_CACHE_HOME": str(root / "cache fixture"),
                "XDG_DATA_HOME": str(root / "data fixture"),
            })
            subprocess.run(
                ["bash", str(ROOT / "claude-worktree"), "cleanup-fixture"],
                cwd=root, env=env, input="Review the fixture.\n", text=True,
                capture_output=True, check=True,
            )
            worktree = root / ".worktrees/cleanup-fixture"
            branch = subprocess.check_output(
                ["git", "branch", "--show-current"], cwd=worktree, text=True
            ).strip()
            self.assertEqual(branch, "cleanup-fixture")
            with (worktree / ".codex/config.toml").open("rb") as file:
                config = tomllib.load(file)
            self.assertEqual(config["permissions"]["sccache-workspace"]["filesystem"], {
                str(root / ".git"): "write",
                str(root / 'cargo "fixture"'): "write",
                str(root / "cache fixture/sccache"): "write",
                str(root / "data fixture/robin_hood"): "write",
            })

    def test_agent_launcher_rejects_path_or_option_names(self):
        for name in ["../escape", "--detach", "nested/branch", ""]:
            result = subprocess.run(
                ["bash", str(ROOT / "claude-worktree"), name],
                cwd=ROOT, input="prompt", text=True, capture_output=True,
            )
            self.assertNotEqual(result.returncode, 0, name)
            self.assertIn("Usage:", result.stdout)

    def test_retired_one_campaign_validators_are_absent(self):
        for name in ["validate_motionstate.sh", "validate_schema15_replacements.sh"]:
            self.assertFalse((ROOT / "scripts" / name).exists())


if __name__ == "__main__":
    unittest.main()
