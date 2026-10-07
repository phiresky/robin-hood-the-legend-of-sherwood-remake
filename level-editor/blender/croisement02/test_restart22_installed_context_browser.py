import unittest
import restart22_prepare_installed_context_browser as h

class InstalledHarnessTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.runner_source = (h.SOURCE / 'run.mjs').read_text()
        cls.states = (h.SOURCE / 'states.mjs').read_text()

    def test_current_harness_has_no_remaining_overlay_and_checks_all_routes(self):
        run, states = h.adapt(self.runner_source, self.states)
        self.assertEqual(run.count('config.overlay'), 1)
        self.assertNotIn('config.overlay', states)
        self.assertIn("join('level-editor/library',entry.contract.path)", states)
        self.assertIn('requiredInstalled.length!==8', run)
        self.assertIn('PASS_INSTALLED_EIGHT_INITIAL_CONTEXTS_NORMAL_HTTP', run)

    def test_changed_contract_read_cannot_silently_keep_stage(self):
        altered = self.states.replace('json(config.overlay[', 'json(otherOverlay[')
        with self.assertRaisesRegex(ValueError, 'cardinality'):
            h.adapt(self.runner_source, altered)

    def test_duplicate_success_status_rejected(self):
        with self.assertRaisesRegex(ValueError, 'cardinality'):
            h.adapt(self.runner_source + "status:'PASS_STAGED_EIGHT_INITIAL_CONTEXTS'", self.states)

    def test_unknown_interception_rejected(self):
        with self.assertRaises(ValueError):
            h.adapt(self.runner_source.replace('Object.hasOwn(config.overlay,pathname)', 'true'), self.states)

if __name__ == '__main__':
    unittest.main()
