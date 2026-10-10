"""Failure-path checks for the private mixed replace/delete publisher."""
import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('transaction', Path(__file__).with_name('restart44_static_transaction.py'))
t = importlib.util.module_from_spec(spec)
spec.loader.exec_module(t)


class TransactionTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.rows = []
        for name, old, new in [('model', b'old model', b'new model'),
                               ('derivative', b'old derivative', None),
                               ('index', b'old index', b'new index')]:
            target = self.root / name
            target.write_bytes(old)
            source = self.root / (name + '.source')
            if new is not None:
                source.write_bytes(new)
            self.rows.append({'target': str(target), 'source': str(source) if new else None,
                              'old_sha256': t.sha(target), 'new_sha256': t.sha(source) if new else None})
        self.folder = self.root / 'transaction'

    def tearDown(self):
        self.tmp.cleanup()

    def run_commit(self, **kwargs):
        return t.commit(self.rows, self.folder, 'test-plan', lambda: None,
                        kwargs.pop('postcheck', lambda: None), **kwargs)

    def test_delete_and_restore(self):
        journal = self.run_commit()
        t.verify(self.rows, 'new_sha256')
        self.assertFalse((self.root / 'derivative').exists())
        t.core.restore(self.rows, self.folder, journal)
        t.verify(self.rows, 'old_sha256')

    def test_failure_after_delete_restores_every_file(self):
        with self.assertRaisesRegex(RuntimeError, 'Injected'):
            self.run_commit(fail_after=2)
        t.verify(self.rows, 'old_sha256')

    def test_postcheck_failure_restores_index_and_deleted_file(self):
        def fail():
            raise RuntimeError('postcheck failure')
        with self.assertRaisesRegex(RuntimeError, 'postcheck'):
            self.run_commit(postcheck=fail)
        t.verify(self.rows, 'old_sha256')

    def test_restore_refuses_external_recreated_derivative(self):
        journal = self.run_commit()
        (self.root / 'derivative').write_bytes(b'other writer')
        with self.assertRaisesRegex(RuntimeError, 'external drift'):
            t.core.restore(self.rows, self.folder, journal)
        self.assertEqual((self.root / 'model').read_bytes(), b'new model')

    def test_corrupt_backup_refused_before_any_restore(self):
        journal = self.run_commit()
        backup = Path(self.rows[1]['backup'])
        backup.chmod(0o644)
        backup.write_bytes(b'corrupt')
        with self.assertRaisesRegex(RuntimeError, 'backup corrupt'):
            t.core.restore(self.rows, self.folder, journal)
        self.assertEqual((self.root / 'model').read_bytes(), b'new model')

    def test_source_drift_never_mutates_targets(self):
        Path(self.rows[0]['source']).write_bytes(b'drift')
        with self.assertRaisesRegex(RuntimeError, 'Incoming mismatch'):
            self.run_commit()
        t.verify(self.rows, 'old_sha256')


if __name__ == '__main__':
    unittest.main()
