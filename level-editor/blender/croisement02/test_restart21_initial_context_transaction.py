import hashlib
import io
import json
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest.mock import patch
import restart21_initial_context_transaction as tx


class TransactionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.library = self.root / 'library'
        self.output = self.root / 'plan'
        self.output.mkdir()
        self.index = self.library / tx.INDEX
        self.index.parent.mkdir(parents=True)
        self.index.write_bytes(b'baseline')
        self.baseline = self.root / 'baseline.json'
        self.baseline.write_bytes(b'baseline')
        self.proposed = self.root / 'proposed.json'
        self.proposed.write_bytes(b'proposed')
        self.records = []
        for i in range(8):
            source = self.root / f'source{i}.json'
            source.write_text(f'contract{i}')
            self.records.append({'source': source.name, 'destination': f'mission-states/contracts/{i}.json', 'sha256': tx.sha(source)})
        self.plan = {'records': self.records, 'baseline_index': {'path': self.baseline.name, 'sha256': tx.sha(self.baseline)},
                     'proposed_index': {'path': self.proposed.name, 'sha256': tx.sha(self.proposed)}}
        (self.output / 'plan.json').write_text(json.dumps(self.plan))
        self.plan_sha = tx.sha(self.output / 'plan.json')
        self.receipt = self.root / 'receipt.json'

    def run_execute(self, verify):
        argv = ['tx', '--execute', '--plan-sha', self.plan_sha, '--receipt', str(self.receipt), '--gate', 'gate.json', '--gate-sha', 'gatehash']
        with patch.object(tx, 'ROOT', self.root), patch.object(tx, 'LIB', self.library), patch.object(tx, 'OUTPUT', self.output), patch.object(tx, 'verify', side_effect=verify), patch.object(tx, 'verify_gate', return_value={}), patch('sys.argv', argv), redirect_stdout(io.StringIO()):
            tx.main()

    def test_index_switch_waits_for_all_verified_contracts(self):
        visits = []
        def check(plan):
            visits.append(self.index.read_bytes())
            if len(visits) == 2:
                for row in plan['records']:
                    self.assertEqual(tx.sha(self.library / row['destination']), row['sha256'])
        self.run_execute(check)
        self.assertEqual(visits, [b'baseline', b'baseline'])
        self.assertEqual(self.index.read_bytes(), b'proposed')
        receipt = tx.read(self.receipt)
        self.assertEqual(receipt['status'], 'INSTALLED_PENDING_NORMAL_HTTP_PROOF')
        self.assertEqual(len(receipt['created']), 8)
        tx.rollback(self.index, b'baseline', tx.sha(self.baseline), tx.sha(self.proposed), receipt, self.plan_sha)
        self.assertEqual(self.index.read_bytes(), b'baseline')
        self.assertTrue(all((self.library / r['destination']).exists() for r in self.records))

    def test_postcopy_guard_failure_preserves_live_catalog(self):
        calls = 0
        def check(_):
            nonlocal calls
            calls += 1
            if calls == 2:
                raise ValueError('runtime changed during copy')
        with self.assertRaisesRegex(ValueError, 'runtime changed'):
            self.run_execute(check)
        self.assertEqual(self.index.read_bytes(), b'baseline')
        self.assertEqual(tx.read(self.receipt)['status'], 'FAILED_BEFORE_SWITCH_OR_EXTERNAL_INDEX')

    def test_foreign_index_not_overwritten(self):
        calls = 0
        def check(_):
            nonlocal calls
            calls += 1
            if calls == 2:
                self.index.write_bytes(b'foreign')
        with self.assertRaisesRegex(ValueError, 'Concurrent index'):
            self.run_execute(check)
        self.assertEqual(self.index.read_bytes(), b'foreign')

    def test_conflicting_immutable_file_never_overwritten(self):
        target = self.library / self.records[0]['destination']
        target.parent.mkdir(parents=True)
        target.write_bytes(b'foreign')
        with self.assertRaisesRegex(ValueError, 'Existing immutable'):
            self.run_execute(lambda _: None)
        self.assertEqual(target.read_bytes(), b'foreign')
        self.assertEqual(self.index.read_bytes(), b'baseline')

    def test_rollback_requires_owned_current_index_and_baseline(self):
        old, new = tx.sha(self.baseline), tx.sha(self.proposed)
        receipt = {'status': 'TRANSACTION_PREPARED', 'plan_sha256': self.plan_sha, 'baseline_index_sha256': old, 'index_sha256': new}
        self.index.write_bytes(b'foreign')
        with self.assertRaisesRegex(ValueError, 'another writer'):
            tx.rollback(self.index, b'baseline', old, new, receipt, self.plan_sha)
        self.index.write_bytes(b'proposed')
        with self.assertRaisesRegex(ValueError, 'replacement index bytes'):
            tx.rollback(self.index, b'wrong baseline', old, new, receipt, self.plan_sha)
        self.assertEqual(self.index.read_bytes(), b'proposed')

    def test_gate_cannot_be_omitted_or_claim_wrong_scope(self):
        with self.assertRaises(ValueError):
            tx.verify_gate(None, None, {}, self.plan_sha)
        gate = self.root / 'gate.json'
        gate.write_text(json.dumps({'status': 'PASS_ROOT_READY_FOR_INITIAL_CONTEXT_PUBLICATION', 'transaction_plan_sha256': self.plan_sha,
                                    'browser_proof_sha256': tx.PROOF_SHA, 'reviewed_runtime_deltas': [], 'scope': 'all-gameplay', 'publication_authorized': True}))
        with self.assertRaisesRegex(ValueError, 'scope/authorization'):
            tx.verify_gate(gate, tx.sha(gate), {'runtime_deltas': []}, self.plan_sha)

    def test_missing_or_failed_browser_assertion_rejected(self):
        checks = [{'name': f'irrelevant{i}', 'pass': True} for i in range(68)]
        with self.assertRaisesRegex(ValueError, 'Missing browser'):
            tx.verify_checks(checks, ['required'])
        checks[0]['pass'] = False
        with self.assertRaisesRegex(ValueError, 'Browser checks failed'):
            tx.verify_checks(checks, ['required'])


if __name__ == '__main__':
    unittest.main()
