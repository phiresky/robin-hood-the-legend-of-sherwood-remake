"""Process-level checks against the real shared FIFO slot protocol; no browser."""
import fcntl
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest

HERE = Path(__file__).resolve().parent
SHARED = HERE.parents[1] / 'refinement/render_slots.py'
WRAPPER = HERE / 'run_full_map_admitted.py'


def wait_for(predicate):
    deadline = time.monotonic() + 8
    while time.monotonic() < deadline:
        if predicate(): return
        time.sleep(0.02)
    raise AssertionError('Child process did not reach expected state')


class AdmissionTests(unittest.TestCase):
    def child(self, pool, exclusive, gated=False):
        source = WRAPPER if exclusive else SHARED
        script = '''
import importlib.util,time,sys
from pathlib import Path
spec=importlib.util.spec_from_file_location('lease',sys.argv[1]);m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
p=Path(sys.argv[2]);exclusive=sys.argv[3]=='True';gated=sys.argv[4]=='True'
if exclusive:
 handles=m.acquire_exclusive(p,2,lambda:(not gated or (p/'ready').exists(),{}),lambda *args:None)
 (p/'exclusive-admitted').touch()
 while not (p/'release-exclusive').exists():time.sleep(.02)
 for handle in handles:handle.close()
else:
 m.acquire(slots=2,directory=p);(p/'normal-admitted').touch();m.release()
'''
        process = subprocess.Popen([sys.executable, '-c', script, str(source), str(pool), str(exclusive), str(gated)], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        def cleanup():
            if process.poll() is None: process.terminate()
            process.communicate(timeout=5)
        self.addCleanup(cleanup)
        return process

    def assert_free(self, pool, indexes):
        for index in indexes:
            with (pool / f'{index}.lock').open('a+') as handle:
                fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)

    def test_exclusive_waits_for_existing_job_and_precedes_later_shared_waiter(self):
        with tempfile.TemporaryDirectory() as tmp:
            pool = Path(tmp)
            with (pool / '1.lock').open('a+') as existing:
                fcntl.flock(existing, fcntl.LOCK_EX)
                exclusive = self.child(pool, True)
                wait_for(lambda: len(list(pool.glob('*.ticket'))) == 1)
                normal = self.child(pool, False)
                wait_for(lambda: len(list(pool.glob('*.ticket'))) == 2)
                self.assertFalse((pool / 'exclusive-admitted').exists())
                self.assert_free(pool, [0])  # No partial reservation while an old holder runs.
                fcntl.flock(existing, fcntl.LOCK_UN)
            wait_for(lambda: (pool / 'exclusive-admitted').exists())
            self.assertFalse((pool / 'normal-admitted').exists())
            with (pool / 'browser.lock').open('a+') as browser:
                with self.assertRaises(BlockingIOError): fcntl.flock(browser, fcntl.LOCK_EX | fcntl.LOCK_NB)
            (pool / 'release-exclusive').touch()
            self.assertEqual(exclusive.wait(timeout=5), 0)
            self.assertEqual(normal.wait(timeout=5), 0)
            self.assertTrue((pool / 'normal-admitted').exists())
            self.assert_free(pool, [0, 1])

    def test_disabled_slots_are_not_part_of_two_slot_admission(self):
        with tempfile.TemporaryDirectory() as tmp:
            pool = Path(tmp)
            with (pool / '2.lock').open('a+') as disabled:
                fcntl.flock(disabled, fcntl.LOCK_EX)
                exclusive = self.child(pool, True)
                wait_for(lambda: (pool / 'exclusive-admitted').exists())
                (pool / 'release-exclusive').touch()
                self.assertEqual(exclusive.wait(timeout=5), 0)

    def test_resource_gate_waits_without_holding_render_slots(self):
        with tempfile.TemporaryDirectory() as tmp:
            pool = Path(tmp)
            exclusive = self.child(pool, True, True)
            wait_for(lambda: len(list(pool.glob('*.ticket'))) == 1)
            self.assert_free(pool, [0, 1])
            self.assertFalse((pool / 'exclusive-admitted').exists())
            (pool / 'ready').touch()
            wait_for(lambda: (pool / 'exclusive-admitted').exists())
            (pool / 'release-exclusive').touch()
            self.assertEqual(exclusive.wait(timeout=5), 0)
            self.assertEqual(list(pool.glob('*.ticket')), [])


if __name__ == '__main__': unittest.main()
