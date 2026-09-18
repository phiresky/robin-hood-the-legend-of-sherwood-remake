#!/usr/bin/env python3
"""Protocol regressions independent of proprietary game data and Rust builds."""
import json
import re
import unittest
import subprocess
import tempfile
from pathlib import Path
from parity_result import PREFIX, RESULT_VERSION, LEGACY_EOF_MARKER, exact_eof, read_result


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

    def test_shared_shell_helpers_use_bounded_decimal_and_preserve_hash_failures(self):
        helper = Path(__file__).parent / "lib/parity_common.sh"
        result = subprocess.run(["bash", "-c", r'''
            set -euo pipefail
            source "$1"
            # `! cmd` never trips errexit, so rejections must fail explicitly.
            reject() {
                if "$@"; then printf 'accepted: %s\n' "$*" >&2; exit 1; fi
            }
            [[ $(normalize_bounded_uint 0008 0008) == 8 ]]
            [[ $(normalize_bounded_uint 000 8) == 0 ]]
            reject normalize_bounded_uint 9 8
            reject normalize_bounded_uint 18446744073709551616 9223372036854775807
            reject normalize_bounded_uint '-1' 8
            reject normalize_bounded_uint '1+1' 8
            reject sha256_file /nonexistent-parity-fixture 2>/dev/null
        ''', "parity-common-test", str(helper)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_shared_runner_bundle_verifier_is_strict(self):
        helper = Path(__file__).parent / "lib/parity_common.sh"
        with tempfile.TemporaryDirectory() as directory:
            result = subprocess.run(["bash", "-c", r'''
                set -euo pipefail
                source "$1"
                make_bundle() {
                    local b=$1
                    mkdir -p -- "$b/lib"
                    for f in original_parity_replay original_parity_replay.remote lib/ld-linux-x86-64.so.2; do
                        printf '#!/bin/sh\n' >"$b/$f"; chmod +x -- "$b/$f"
                    done
                    printf 'NATIVE_CONVERSION_PROTOCOL=2\n' >"$b/PROVENANCE.txt"
                    printf 'ld => %s/lib/ld-linux-x86-64.so.2 (0x1)\n' "$b" >"$b/LOADER_LIST.txt"
                    (cd -- "$b" && sha256sum lib/ld-linux-x86-64.so.2 >LIB_SHA256SUMS \
                        && sha256sum original_parity_replay original_parity_replay.remote \
                            PROVENANCE.txt LOADER_LIST.txt LIB_SHA256SUMS >SHA256SUMS)
                }
                # `! cmd` never trips errexit, so rejections must fail explicitly.
                reject() {
                    if "$@"; then printf 'accepted: %s\n' "$*" >&2; exit 1; fi
                }
                good="$2/with space/good"; make_bundle "$good"
                verify_runner_bundle "$good" "$good"
                verify_runner_bundle_identity "$good" "$(runner_bundle_digest "$good")" \
                    "$(sha256_file "$good/original_parity_replay")"
                reject verify_runner_bundle_identity "$good" "$(printf '0%.0s' {1..64})" 2>/dev/null
                reject verify_runner_bundle "$good" "" 2>/dev/null
                reject verify_runner_bundle "$good" "$2/elsewhere" 2>/dev/null

                traversal="$2/traversal"; make_bundle "$traversal"
                printf '%064d  ../outside\n' 0 >>"$traversal/LIB_SHA256SUMS"
                reject verify_runner_bundle "$traversal" "$traversal" 2>"$2/traversal.err"
                grep -Fq 'unsafe bundle checksum path: ../outside' "$2/traversal.err"

                dotdot="$2/dotdot"; make_bundle "$dotdot"
                printf 'libc => %s/lib/../../escape.so (0x2)\n' "$dotdot" >>"$dotdot/LOADER_LIST.txt"
                reject verify_runner_bundle "$dotdot" "$dotdot" 2>"$2/dotdot.err"
                grep -Fq 'loader proof resolves outside' "$2/dotdot.err"

                linked="$2/linked"; make_bundle "$linked"
                ln -s /etc/passwd "$linked/lib/escape"
                reject verify_runner_bundle "$linked" "$linked" 2>"$2/linked.err"
                grep -Fq 'contains a symlink' "$2/linked.err"

                mkdir -p -- "$2/audit/logs"
                : >"$2/audit/logs/k.attempt-0002.log.in-progress"
                attempt_begin "$2/audit/logs" k 2 "$2/audit/k.status"
                [[ "$attempt_number" == 3 && "$(cat "$2/audit/k.status")" == $'running\t3\tk.attempt-0003.log' ]]
                attempt_finish "$2/audit/k.status" 0
                [[ -f "$2/audit/logs/k.attempt-0003.log" ]]
                attempt_read_status "$2/audit/k.status"
                [[ "$attempt_prior_status" == 0 && "$attempt_prior_number" == 3 ]]
                printf 'extra\n' >>"$2/audit/k.status"
                ! attempt_read_status "$2/audit/k.status"
            ''', "parity-common-bundle-test", str(helper), directory],
                capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()
