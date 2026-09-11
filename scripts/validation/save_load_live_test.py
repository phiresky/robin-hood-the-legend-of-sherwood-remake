#!/usr/bin/env python3
"""Pure assertion tests; the native driver remains a separate integration gate."""

import copy
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from save_load_live import recording_bounds, restored_authority, verify_replay, verify_restored


class SaveLoadAssertions(unittest.TestCase):
    def test_chunked_recording_bounds_and_rejected_bad_boundaries(self):
        chunks = [
            {"file": "00000000.rhrec.jsonl", "previous": None, "first_ordinal": 0, "loaded_save": None},
            {"file": "00000001.rhrec.jsonl", "previous": "00000000.rhrec.jsonl", "first_ordinal": 139, "loaded_save": {"marker": 56}},
        ]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest = {"version": 1, "chunks": chunks}
            (root / "mission.json").write_text(json.dumps(manifest))
            (root / chunks[0]["file"]).write_text('\n'.join(map(json.dumps, [
                {"chunk": chunks[0]}, {"f": 0}, {"f": 138}])))
            path = root / chunks[1]["file"]
            rows = [{"chunk": chunks[1]}, {"f": 139, "t": [{"kind": "state_load"}]}, {"f": 250}]
            path.write_text('\n'.join(map(json.dumps, rows)))
            self.assertEqual(recording_bounds(root), (139, 250))
            for transitions in ([], [{"kind": "state_load"}, {"kind": "other"}]):
                invalid = copy.deepcopy(rows)
                invalid[1]["t"] = transitions
                if transitions:
                    invalid.append({"f": 200, "t": [{"kind": "state_load"}]})
                path.write_text('\n'.join(map(json.dumps, invalid)))
                with self.assertRaisesRegex(AssertionError, "state_load"):
                    recording_bounds(root)
            path.write_text('\n'.join(map(json.dumps, rows)))
            chunks[1]["previous"] = None
            (root / "mission.json").write_text(json.dumps(manifest))
            with self.assertRaisesRegex(AssertionError, "linkage"):
                recording_bounds(root)

    def test_modes_that_skip_gate_are_rejected(self):
        for extra in (["--replay-file", "missing"], ["--bootstrap-probe"]):
            with self.subTest(extra=extra):
                result = subprocess.run([sys.executable, str(Path(__file__).with_name("frame_steps_live.py")),
                    "--binary", "missing", "--data", "missing", "--snapshot", "test",
                    "--evidence", "missing", "--save-load", *extra], capture_output=True, text=True)
                self.assertEqual(result.returncode, 2)
                self.assertIn("--save-load requires", result.stderr)

    def reconciliation_fixture(self):
        saved = {"players": {"seats": [{"selection": [{"Pc": 198}], "selected_action": "NoAction"}]},
                 "orders": {"messenger": {"queue": []}},
                 "scripts": {"mission": {"script_effects": {"ordered": []}}}}
        restored = copy.deepcopy(saved)
        restored["orders"]["messenger"]["queue"] = [
            {"arg1": 0, "arg2": 0, "msg_type": {"Simple": "Stature"}, "value": 0},
            {"arg1": 0, "arg2": 0, "msg_type": {"Pc": ["SelectAction", {"Pc": 198}]}, "value": 0}]
        restored["scripts"]["mission"]["script_effects"]["ordered"] = [{"Presentation": "UpdateInformationBars"}]
        return saved, restored

    def test_exact_reconciliation_only_without_mutation(self):
        saved, restored = self.reconciliation_fixture()
        original = copy.deepcopy(restored)
        self.assertEqual(saved, restored_authority(saved, restored))
        self.assertEqual(original, restored)

    def test_unexpected_queue_message_rejected(self):
        saved, restored = self.reconciliation_fixture()
        restored["orders"]["messenger"]["queue"].append({"unknown": True})
        with self.assertRaisesRegex(AssertionError, "messenger"):
            restored_authority(saved, restored)

    def test_wrong_selected_pc_rejected(self):
        saved, restored = self.reconciliation_fixture()
        restored["orders"]["messenger"]["queue"][1]["msg_type"]["Pc"][1] = {"Pc": 199}
        with self.assertRaisesRegex(AssertionError, "messenger"):
            restored_authority(saved, restored)

    def test_unknown_script_effect_rejected(self):
        saved, restored = self.reconciliation_fixture()
        restored["scripts"]["mission"]["script_effects"]["ordered"].append({"External": "unknown"})
        with self.assertRaisesRegex(AssertionError, "presentation"):
            restored_authority(saved, restored)

    def test_identical_restored_authority(self):
        verify_restored({"frame": 50, "seats": [1]}, {"frame": 50, "seats": [1]}, 50, 50)

    def test_wrong_restored_frame(self):
        with self.assertRaisesRegex(AssertionError, "restored frame"):
            verify_restored({}, {}, 50, 51)

    def test_changed_authority(self):
        with self.assertRaisesRegex(AssertionError, "state differs"):
            verify_restored({"seats": [1]}, {"seats": [2]}, 50, 50)

    def test_post_restore_hash_and_finish(self):
        verify_replay("Replay hash OK @ frame 100\nheadless replay finished", 50, 150)

    def test_bootstrap_only_hash_is_not_enough(self):
        with self.assertRaisesRegex(AssertionError, "beyond"):
            verify_replay("Replay hash OK @ frame 25\nheadless replay finished", 50, 150)

    def test_hash_outside_recording_is_not_enough(self):
        with self.assertRaisesRegex(AssertionError, "beyond"):
            verify_replay("Replay hash OK @ frame 200\nheadless replay finished", 50, 150)

    def test_desync_even_with_other_verified_hash(self):
        with self.assertRaisesRegex(AssertionError, "desynchronized"):
            verify_replay("Replay hash OK @ frame 100\nReplay desync\nheadless replay finished", 50, 150)

    def test_unfinished_replay(self):
        with self.assertRaisesRegex(AssertionError, "did not finish"):
            verify_replay("Replay hash OK @ frame 100", 50, 150)


if __name__ == "__main__":
    unittest.main()
