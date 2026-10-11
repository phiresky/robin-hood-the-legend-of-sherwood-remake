import copy
import unittest
from unittest.mock import patch
import restart58_publish_holes_mounds as publisher
from restart58_publish_holes_mounds import merge_catalog

class CatalogRebaseTests(unittest.TestCase):
    def setUp(self):
        self.additions = [{'id': f'reviewed-{i}', 'mission': 'Emb05'} for i in range(62)]

    def test_preserves_concurrent_entries_metadata_and_input(self):
        before = {'version': 1, 'extension': {'keep': True}, 'entries': [{'id': 'york-new'}, {'id': 'climbing-new'}]}
        original = copy.deepcopy(before)
        after = merge_catalog(before, self.additions)
        self.assertEqual(before, original)
        self.assertEqual(after['entries'][:2], original['entries'])
        self.assertEqual(after['extension'], original['extension'])
        self.assertEqual(len(after['entries']), 64)

    def test_rejects_conflicting_or_already_installed_entries(self):
        with self.assertRaises(Exception):
            merge_catalog({'entries': [{'id': 'reviewed-0', 'mission': 'other'}]}, self.additions)

    def test_rejects_duplicate_existing_and_incomplete_scope(self):
        with self.assertRaises(Exception):
            merge_catalog({'entries': [{'id': 'x'}, {'id': 'x'}]}, self.additions)
        with self.assertRaises(Exception):
            merge_catalog({'entries': []}, self.additions[:-1])


class RuntimeGuardTests(unittest.TestCase):
    def test_scoped_receipt_accepts_only_exact_reviewed_current_bytes(self):
        receipt_path = publisher.AUTHORITY.parent/'test-root-receipt.json'
        receipt_pin = {'path': str(receipt_path), 'sha256': 'receipt-sha'}
        receipt = {'status': 'ROOT_APPROVED_SCOPED_STATE_PUBLICATION', 'authority_sha256': publisher.AUTHORITY_SHA,
                   'transaction_sha256': publisher.STAGE_SHA, 'browser_sha256': publisher.PROOF_SHA, 'postinstall_ui_required': True}
        rows = [{'path': path, 'tested_sha256': 'old-'+path, 'current_sha256': 'new-'+path,
                 'diff': {'path': path+'.diff', 'sha256': 'diff-sha'}} for path in publisher.SCOPED_PATHS]
        authority = {'transaction_sha256': publisher.STAGE_SHA, 'browser': {'sha256': publisher.PROOF_SHA},
                     'allowed_drift': rows, 'dependency_evidence': [{'path': 'helper.ts', 'sha256': 'helper-sha'}]}
        hashes = {receipt_path: 'receipt-sha', publisher.AUTHORITY: publisher.AUTHORITY_SHA, publisher.ROOT/'helper.ts': 'helper-sha'}
        for row in rows:
            hashes[publisher.ROOT/row['path']] = row['current_sha256']
            hashes[publisher.ROOT/row['diff']['path']] = 'diff-sha'
        snapshot = {row['path']: row['tested_sha256'] for row in rows}
        with patch.object(publisher, 'sha', side_effect=lambda path: hashes[path]), patch.object(publisher, 'read', side_effect=lambda path: receipt if path == receipt_path else authority):
            self.assertEqual(publisher.scoped_runtime(receipt_pin, snapshot), {r['path']: r['current_sha256'] for r in rows})
            hashes[publisher.ROOT/'helper.ts'] = 'later-helper'
            with self.assertRaisesRegex(RuntimeError, 'Scope dependency evidence drift'):
                publisher.scoped_runtime(receipt_pin, snapshot)
            hashes[publisher.ROOT/'helper.ts'] = 'helper-sha'
            receipt['status'] = 'PROPOSED'
            with self.assertRaisesRegex(RuntimeError, 'Root scope approval missing'):
                publisher.scoped_runtime(receipt_pin, snapshot)

    def test_disclosed_drift_is_not_a_runtime_guard_waiver(self):
        viewport = 'level-editor/app/src/editor-viewport.ts'
        changed = 'level-editor/shared/src/example-runtime.ts'
        snapshot = {viewport: 'viewport-sha', changed: 'reviewed-sha'}
        proof = {'status': 'PASS', 'transaction_sha256': publisher.STAGE_SHA,
                 'checks': dict.fromkeys(('all_62_controls', 'receiver_move_rejected', 'receiver_restored', 'map_retirement', 'representative_controls'), True),
                 'guard_after': {'ready': False, 'actual': {'dx': 2}},
                 'guard_before': {'actual': {'dx': 1}}, 'runtime': snapshot,
                 'evidence': {}, 'live_source_postflight': {'drift': [{'path': changed}]}}
        hashes = {publisher.HELPER: 'a71796d93dbfbc8fd07ebd6deebf8bc0177e7e599a85b9233c43147fd38a30e4',
                  publisher.STAGE/'transaction.json': publisher.STAGE_SHA,
                  publisher.PROOF: publisher.PROOF_SHA,
                  publisher.ROOT/viewport: 'viewport-sha', publisher.ROOT/changed: 'later-sha'}
        with patch.object(publisher, 'sha', side_effect=lambda path: hashes[path]), patch.object(publisher, 'read', side_effect=lambda path: proof if path == publisher.PROOF else snapshot):
            with self.assertRaisesRegex(RuntimeError, 'Proven runtime changed: '+changed):
                publisher.guards()

if __name__ == '__main__':
    unittest.main()
