import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from asset_index import write_asset_index
from refresh_index_editor import refresh_index_editor


class RefreshIndexEditorTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.asset = self.root / 'house'
        self.asset.mkdir()
        (self.asset / 'model.glb').write_bytes(b'model')
        (self.asset / 'lossy.glb').write_bytes(b'lossy')
        (self.asset / 'lossy.glb.receipt.json').write_text(json.dumps({
            'source': hashlib.sha256(b'model').hexdigest(),
            'output': hashlib.sha256(b'lossy').hexdigest(),
        }))
        self.descriptor = {'id': 'house', 'name': 'House', 'source_map': 'Example',
                           'model': 'model.glb', 'gameplay': {'version': 1}}
        (self.asset / 'asset.json').write_text(json.dumps(self.descriptor))
        self.index = write_asset_index(self.root)
        self.index['assets'][0]['editor']['gameplay'] = {'version': 0}
        (self.root / 'index.json').write_text(json.dumps(self.index))
        self.before = (self.root / 'index.json').read_bytes()

    def test_audit_preserves_cache_and_repair_restores_pinned_gameplay(self):
        report = refresh_index_editor(self.root)
        self.assertEqual(report['changes'], [{'asset': 'house', 'fields': ['gameplay']}])
        self.assertFalse(report['repaired'])
        self.assertEqual((self.root / 'index.json').read_bytes(), self.before)
        report = refresh_index_editor(self.root, repair=True)
        self.assertTrue(report['repaired'])
        actual = json.loads((self.root / 'index.json').read_bytes())
        expected = json.loads(self.before)
        expected['assets'][0]['editor']['gameplay'] = self.descriptor['gameplay']
        self.assertEqual(actual, expected)
        self.assertEqual(refresh_index_editor(self.root)['changes'], [])

    def test_unpublished_descriptor_changes_are_not_adopted(self):
        (self.asset / 'asset.json').write_text(json.dumps({**self.descriptor, 'name': 'Draft'}))
        with self.assertRaisesRegex(ValueError, 'Descriptor pin must be published first'):
            refresh_index_editor(self.root, repair=True)
        self.assertEqual((self.root / 'index.json').read_bytes(), self.before)

    def test_invalid_derivative_blocks_the_changed_entry(self):
        (self.asset / 'model.glb').write_bytes(b'changed model')
        with self.assertRaisesRegex(ValueError, 'lossy receipt'):
            refresh_index_editor(self.root, repair=True)
        self.assertEqual((self.root / 'index.json').read_bytes(), self.before)
        self.assertEqual(list(self.root.glob('.index-editor-*.tmp')), [])


if __name__ == '__main__':
    unittest.main()
