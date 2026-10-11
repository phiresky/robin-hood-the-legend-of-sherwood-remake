import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
import restart73_hall_catalog_transaction as tx

class HallPublicationTests(unittest.TestCase):
    def test_private_stage_validation_never_reads_canonical(self):
        original_open = Path.open
        def guarded(path, *args, **kwargs):
            self.assertFalse(path.is_relative_to(tx.LIB), 'Canonical read forbidden during writer window')
            return original_open(path, *args, **kwargs)
        with patch.object(Path, 'open', guarded):
            stage, contract = tx.stage_plan()
        self.assertEqual(len(stage['records']), 15)
        self.assertEqual(contract['cover_visibility'][0]['source']['index'], 2)

    def test_latest_catalog_rebase_preserves_all_unrelated_data(self):
        before = {'version': 1, 'new_metadata': {'v': 6}, 'entries': [{'id': 'later-agent', 'v': [2, 4]}]}
        entry = {'id': 'hall', 'contract': {'sha256': 'x'}}
        result = tx.merged_catalog(before, entry)
        self.assertEqual(result, {**before, 'entries': [*before['entries'], entry]})
        self.assertEqual(before['entries'], [{'id': 'later-agent', 'v': [2, 4]}])
        with self.assertRaisesRegex(Exception, 'already installed'):
            tx.merged_catalog(result, entry)

    def test_missing_normal_ui_proof_refuses(self):
        with self.assertRaisesRegex(Exception, 'Exact reviewed normal UI proof'):
            tx.proof(None, None, {})

    def test_failed_transaction_restores_backups_and_removes_new_resource(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            old = root / 'catalog.json'; old.write_bytes(b'current unrelated catalog')
            payload = root / 'new-resource'; payload.write_bytes(b'exact approved bytes')
            merged = root / 'merged-catalog'; merged.write_bytes(b'merged preserving unrelated')
            destination = root / 'installed-resource'
            records = [
                {'source': str(payload), 'target': str(destination), 'old_sha256': None, 'new_sha256': tx.sha(payload)},
                {'source': str(merged), 'target': str(old), 'old_sha256': tx.sha(old), 'new_sha256': tx.sha(merged)},
            ]
            with self.assertRaisesRegex(RuntimeError, 'Injected failure'):
                tx.io.commit(records, root / 'transaction', 'pinned-plan', lambda: None, lambda: None, fail_after=2)
            self.assertEqual(old.read_bytes(), b'current unrelated catalog')
            self.assertFalse(destination.exists())
            journal = json.loads((root / 'transaction/journal.json').read_text())
            self.assertEqual(journal['phase'], 'ROLLED_BACK')

    def test_drift_before_backup_refuses_without_touching_external_data(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); target = root / 'live'; target.write_bytes(b'external changed data')
            incoming = root / 'incoming'; incoming.write_bytes(b'candidate')
            row = {'source': str(incoming), 'target': str(target), 'old_sha256': 'a'*64, 'new_sha256': tx.sha(incoming)}
            with self.assertRaisesRegex(Exception, 'Drift before backup'):
                tx.io.commit([row], root / 'transaction', 'plan', lambda: None, lambda: None)
            self.assertEqual(target.read_bytes(), b'external changed data')

if __name__ == '__main__': unittest.main()
