#!/usr/bin/env python3
"""Check suite dispatch/coverage without compiling application dependencies."""
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import tomllib
import unittest

ROOT = Path(__file__).resolve().parents[1]
RUST_SUITES = (
    "core", "scripting-llvm", "engine", "assets", "protocols", "services", "parity",
    "client", "client-release", "tools", "wasm", "gpu", "gpu-gl", "host",
)


class QualitySuitesTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.log = self.directory / "calls.jsonl"
        cargo = self.directory / "cargo"
        cargo.write_text(
            "#!/usr/bin/env python3\n"
            "import json, os, sys\n"
            "with open(os.environ['QUALITY_TEST_CALLS'], 'a') as output:\n"
            "    output.write(json.dumps(sys.argv[1:]) + '\\n')\n",
            encoding="utf-8",
        )
        cargo.chmod(0o755)
        self.environment = dict(os.environ)
        self.environment["PATH"] = str(self.directory) + os.pathsep + os.environ["PATH"]
        self.environment["QUALITY_TEST_CALLS"] = str(self.log)
        self.gpu_backend = self.directory / "gpu-backend"
        self.environment["QUALITY_TEST_GPU_BACKEND"] = str(self.gpu_backend)
        xvfb = self.directory / "xvfb-run"
        xvfb.write_text(
            '#!/bin/sh\n[ "$1" = "-a" ] || exit 64\nshift\n'
            'printf "%s\\n" "${WGPU_BACKEND:?}" > "$QUALITY_TEST_GPU_BACKEND"\n'
            'exec "$@"\n'
        )
        xvfb.chmod(0o755)
        self.environment.pop("ROBINHOOD_DATA_DIR", None)

    def run_suite(self, suite, expected=0):
        result = subprocess.run(
            ["bash", str(ROOT / "scripts/check-quality.sh"), suite],
            cwd=self.directory, env=self.environment, capture_output=True, text=True,
            timeout=15,
        )
        self.assertEqual(result.returncode, expected, result.stderr)

    def calls(self):
        return [json.loads(line) for line in self.log.read_text().splitlines()]

    def test_every_workspace_crate_has_an_explicit_gate(self):
        for suite in RUST_SUITES:
            self.run_suite(suite)
        packages = {
            call[index + 1]
            for call in self.calls()
            for index, argument in enumerate(call)
            if argument == "-p"
        }
        workspace = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]
        expected = {
            tomllib.loads((ROOT / member / "Cargo.toml").read_text())["package"]["name"]
            for member in workspace["members"]
        }
        self.assertEqual(packages, expected)
        for call in self.calls():
            self.assertIn("--locked", call)
            self.assertNotIn("--all-features", call)
            self.assertNotIn("--target-dir", call)

    def test_client_build_is_a_separate_command(self):
        self.run_suite("client")
        self.assertEqual([call[0] for call in self.calls()], ["test", "build"])

    def test_signer_workspace_dependency_boundary_including_target_and_build_edges(self):
        workspace = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]
        manifests = {}
        for member in workspace["members"]:
            manifest = tomllib.loads((ROOT / member / "Cargo.toml").read_text())
            manifests[manifest["package"]["name"]] = manifest
        visited = set()

        def visit(package):
            if package in visited:
                return
            visited.add(package)
            self.assertIn(package, {"robin_identity_signer", "robin_run_protocol"})
            manifest = manifests[package]
            tables = [manifest, *manifest.get("target", {}).values()]
            for table in tables:
                for kind in ("dependencies", "build-dependencies"):
                    for name, spec in table.get(kind, {}).items():
                        if isinstance(spec, dict) and spec.get("workspace"):
                            spec = workspace["dependencies"][name]
                        dependency = spec.get("package", name) if isinstance(spec, dict) else name
                        self.assertNotIn(dependency, {"wgpu", "winit", "cpal", "rodio"})
                        if dependency in manifests:
                            visit(dependency)

        visit("robin_identity_signer")
        self.assertEqual(visited, {"robin_identity_signer", "robin_run_protocol"})

    def test_gpu_gate_explicitly_selects_ignored_execution_test(self):
        self.run_suite("gpu")
        self.assertEqual(self.calls()[0][-3:], ["--", "--ignored", "--exact"])

    def test_gl_gate_uses_an_owned_display_and_explicit_backend(self):
        self.run_suite("gpu-gl")
        self.assertEqual(self.gpu_backend.read_text(), "gl\n")
        self.assertEqual(self.calls()[0][-3:], ["--", "--ignored", "--exact"])

    def test_missing_fixture_configuration_fails_before_cargo(self):
        self.run_suite("fixtures-demo", expected=1)
        self.run_suite("fixtures-fullgame", expected=1)
        self.assertFalse(self.log.exists())

    def test_host_gate_enables_and_selects_the_real_backend(self):
        self.run_suite("host")
        call = self.calls()[0]
        self.assertEqual(call[call.index("--features") + 1], "hardware-info")
        self.assertEqual(call[-3:], ["--", "--ignored", "--exact"])

    def test_unwind_gate_explicitly_uses_llvm(self):
        self.run_suite("scripting-llvm")
        calls = self.calls()
        self.assertEqual(len(calls), 2)
        for call in calls:
            self.assertTrue(call[call.index("--config") + 1].endswith('.codegen-backend="llvm"'))
        self.assertEqual(calls[0][-2:], ["--", "--ignored"])

    def test_unknown_suite_fails(self):
        self.run_suite("not-a-suite", expected=2)
        self.assertFalse(self.log.exists())

    def test_browser_requires_an_explicit_executable(self):
        self.environment.pop("CHROME", None)
        self.run_suite("editor-browser", expected=1)

    def test_lifecycle_suites_fail_for_missing_provisioning_without_cargo(self):
        self.environment["ROBIN_LIFECYCLE_EVIDENCE"] = str(self.directory / "browser-evidence")
        self.environment.pop("CHROME", None)
        self.run_suite("browser-audio", expected=1)
        self.environment["ROBIN_LIFECYCLE_EVIDENCE"] = str(self.directory / "native-evidence")
        self.environment.pop("ROBIN_LIFECYCLE_BINARY", None)
        self.run_suite("native-lifecycle", expected=1)
        self.assertFalse(self.log.exists())
        for name in ("browser-evidence", "native-evidence"):
            result = json.loads((self.directory / name / "summary.json").read_text())
            self.assertFalse(result["completed"])
            self.assertIn("error", result)

    def test_browser_failure_stops_even_an_uncooperative_preview(self):
        preview_pid_file = self.directory / "preview.pid"
        self.environment["QUALITY_TEST_PREVIEW_PID"] = str(preview_pid_file)
        chrome = self.directory / "chrome"
        chrome.write_text("#!/bin/sh\nprintf 'fixture Chrome\\n'\n")
        chrome.chmod(0o755)
        self.environment["CHROME"] = str(chrome)
        pnpm = self.directory / "pnpm"
        pnpm.write_text(
            "#!/usr/bin/env python3\n"
            "import os, pathlib, signal, sys, time\n"
            "command = sys.argv[-1]\n"
            "if command == 'serve:browser':\n"
            "    signal.signal(signal.SIGTERM, signal.SIG_IGN)\n"
            "    pathlib.Path(os.environ['QUALITY_TEST_PREVIEW_PID']).write_text(str(os.getpid()))\n"
            "    while True: time.sleep(1)\n"
            "sys.exit(42 if command == 'test:browser' else 0)\n"
        )
        pnpm.chmod(0o755)
        curl = self.directory / "curl"
        curl.write_text('#!/bin/sh\ntest -s "$QUALITY_TEST_PREVIEW_PID"\n')
        curl.chmod(0o755)
        try:
            self.run_suite("editor-browser", expected=42)
            preview_pid = int(preview_pid_file.read_text())
            with self.assertRaises(ProcessLookupError):
                os.kill(preview_pid, 0)
        finally:
            # Keep a failed cleanup regression from leaking its own fixture.
            if preview_pid_file.exists():
                try:
                    os.killpg(int(preview_pid_file.read_text()), signal.SIGKILL)
                except ProcessLookupError:
                    pass


if __name__ == "__main__":
    unittest.main()
