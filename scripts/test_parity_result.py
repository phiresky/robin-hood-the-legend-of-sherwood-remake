#!/usr/bin/env python3
"""Protocol regressions independent of proprietary game data and Rust builds."""
import json
import re
import unittest
import subprocess
import tempfile
from pathlib import Path
from parity_result import PREFIX, RESULT_VERSION, LEGACY_EOF_MARKER, exact_eof, read_result
from parity_campaign import DEPENDENCIES, load
from run_parity_fixture_gate import snapshot_runner, digest


def result_log(**changes):
    result = dict(result_version=1, trace_path="/trace.jsonl.zst",
                  native_trace_sha256="a"*64, executable_path="/runner",
                  executable_sha256="b"*64, expected_frames=100, processed_frames=100,
                  expected_final_frame=140, final_frame=140, terminator_validated=True,
                  divergent_frames=0, outcome="exact_eof",
                  capabilities=dict(policy_version=1, trace_schema=16, native_version=68,
                                    exceptions=[]))
    result.update(changes)
    return PREFIX + json.dumps(result)


class ResultTests(unittest.TestCase):
    def test_python_protocol_matches_rust_constants(self):
        rust = (Path(__file__).resolve().parents[1] / "crates/robin_parity/src/result.rs").read_text()
        for name, value in (("RESULT_PREFIX", PREFIX), ("LEGACY_EOF_MARKER", LEGACY_EOF_MARKER)):
            match = re.search(rf'pub const {name}: &str = ("[^"\n]*");', rust)
            self.assertIsNotNone(match, name)
            self.assertEqual(json.loads(match[1]), value)
        match = re.search(r'pub const RESULT_VERSION: u32 = (\d+);', rust)
        self.assertIsNotNone(match)
        self.assertEqual(int(match[1]), RESULT_VERSION)

    def test_fixture_gate_pins_executable_across_rebuilds(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "build-output"
            snapshot = Path(directory) / "audit-runner"
            source.write_bytes(b"first build")
            source.chmod(0o755)
            expected = snapshot_runner(source, snapshot)
            source.write_bytes(b"replacement build")
            self.assertEqual(digest(snapshot), expected)
            self.assertNotEqual(digest(source), expected)
            self.assertEqual(snapshot.stat().st_mode & 0o777, 0o555)

    def test_success_does_not_depend_on_human_wording(self):
        self.assertTrue(exact_eof("new human wording\n" + result_log()))
        self.assertFalse(exact_eof(LEGACY_EOF_MARKER))
        self.assertTrue(exact_eof(LEGACY_EOF_MARKER, allow_legacy=True))

    def test_no_prefix_or_partial_frames_can_claim_eof(self):
        for change in (dict(processed_frames=99), dict(final_frame=139),
                       dict(terminator_validated=False), dict(divergent_frames=1),
                       dict(outcome="divergence"), dict(outcome="incomplete")):
            with self.subTest(change=change):
                self.assertFalse(exact_eof(result_log(**change)))

    def test_invalid_new_result_never_falls_back_to_old_sentence(self):
        for change in (dict(result_version=2), dict(result_version=True),
                       dict(processed_frames=True), dict(processed_frames=-1),
                       dict(native_trace_sha256="invalid"), dict(terminator_validated=1),
                       dict(capabilities={}), dict(outcome="success")):
            with self.subTest(change=change), self.assertRaises(ValueError):
                exact_eof(LEGACY_EOF_MARKER + "\n" + result_log(**change), allow_legacy=True)

    def test_duplicate_and_malformed_records_are_rejected(self):
        for log in (result_log()+"\n"+result_log(), PREFIX+"{", PREFIX+"[]"):
            with self.subTest(log=log), self.assertRaises(ValueError):
                read_result(log)

    def test_trace_identity_is_bound(self):
        self.assertTrue(exact_eof(result_log(), trace=Path("/trace.jsonl.zst")))
        with self.assertRaises(ValueError):
            exact_eof(result_log(), trace=Path("/other.jsonl.zst"))

    def test_capability_exceptions_need_scope_and_removal_condition(self):
        capabilities = read_result(result_log())["capabilities"]
        capabilities["exceptions"] = [dict(id="draw-viewport", scope="sprite cache", removal_condition="record viewport")]
        self.assertTrue(exact_eof(result_log(capabilities=capabilities)))
        del capabilities["exceptions"][0]["removal_condition"]
        with self.assertRaises(ValueError):
            exact_eof(result_log(capabilities=capabilities))

    def test_frozen_campaign_retains_deployed_script_hash(self):
        manifest = load(Path(__file__).parent / "parity-campaigns/schema16-20260824.json")
        self.assertEqual(manifest["profiles"]["existing_corpora"]["expected_prepass_script_sha"],
                         "d2b7fd1eb29a921655a3aec49617ca5c48e70cbf990f94593490ba17631a074b")

    def test_campaign_binds_reusable_validator_dependency(self):
        scripts = Path(__file__).parent
        manifest = load(scripts / "parity-campaigns/schema16-20260824.json")
        manifest["script_dependencies"] = {name: "0" * 64 for name in DEPENDENCIES}
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "campaign.json"
            path.write_text(json.dumps(manifest))
            result = subprocess.run(["python3", str(scripts / "parity_campaign.py"),
                                     str(path), "--profile", "existing_corpora"],
                                    capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("campaign dependency hash mismatch", result.stderr)

    def test_shared_shell_helpers_use_bounded_decimal_and_preserve_hash_failures(self):
        helper = Path(__file__).parent / "lib/parity_common.sh"
        result = subprocess.run(["bash", "-c", r'''
            set -euo pipefail
            source "$1"
            [[ $(normalize_bounded_uint 0008 0008) == 8 ]]
            [[ $(normalize_bounded_uint 000 8) == 0 ]]
            ! normalize_bounded_uint 9 8
            ! normalize_bounded_uint 18446744073709551616 9223372036854775807
            ! normalize_bounded_uint '-1' 8
            ! normalize_bounded_uint '1+1' 8
            ! sha256_file /nonexistent-parity-fixture
        ''', "parity-common-test", str(helper)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()
