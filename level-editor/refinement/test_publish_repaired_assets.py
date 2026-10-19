"""Publication transactions reject stale files and restore partially written assets."""
import importlib.util
import json
import copy
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT))
spec = importlib.util.spec_from_file_location('publish_repaired_assets', ROOT / 'publish-repaired-assets.py')
publisher = importlib.util.module_from_spec(spec)
spec.loader.exec_module(publisher)


class RepairPublicationTest(unittest.TestCase):
    def test_room_connections_preserve_scene_bytes_and_reject_stale_or_missing_owners(self):
        original = b'{"height":1.0000000000000001,"placements":[{"id":"a","assets":["hall"]},{"id":"b","assets":["tower"]}]}\n'
        connection = {'id': 'link',
                      'from': {'placement': 'a', 'asset': 'hall', 'interior': 'room'},
                      'to': {'placement': 'b', 'asset': 'tower', 'interior': 'room'}}
        review = {'sceneSha256': publisher.digest(original), 'interiorConnections': [connection]}
        edits = {asset: {'gameplay': {'interiors': [{'id': 'room'}]}} for asset in ('hall', 'tower')}
        result = publisher.add_reviewed_connections(original, review, edits)
        self.assertTrue(result.startswith(original.rstrip()[:-1]))
        self.assertEqual(json.loads(result)['interiorConnections'], [connection])
        with self.assertRaisesRegex(AssertionError, 'Stale connection scene'):
            publisher.add_reviewed_connections(original + b' ', review, edits)
        missing = copy.deepcopy(review)
        missing['interiorConnections'][0]['to']['interior'] = 'absent'
        with self.assertRaisesRegex(AssertionError, 'Missing connection room'):
            publisher.add_reviewed_connections(original, missing, edits)
        missing['interiorConnections'][0]['to']['placement'] = 'absent'
        with self.assertRaisesRegex(AssertionError, 'Missing connection placement'):
            publisher.add_reviewed_connections(original, missing, edits)
        existing = dict(review, sceneSha256=publisher.digest(result))
        with self.assertRaisesRegex(AssertionError, 'existing scene connections'):
            publisher.add_reviewed_connections(result, existing, edits)

    def test_only_reviewed_part_appearance_may_change(self):
        previous = {'model': 'model.glb', 'parts': [{'node': 'fixed'}, {'node': 'removed'}]}
        edit = {'gameplay': {'version': 1},
                'partAppearances': {'removed': {'hide': ['remove']}}}
        replacement = {'model': 'model.glb', 'gameplay': {'version': 1},
                       'parts': [{'node': 'fixed'}, {'node': 'removed',
                                  'appearance': {'hide': ['remove']}}]}
        publisher.verify_descriptor_edit(previous, replacement, edit)
        self.assertNotIn('appearance', previous['parts'][1])
        replacement['parts'][0]['appearance'] = {'hide': ['remove']}
        with self.assertRaisesRegex(AssertionError, 'outside the reviewed'):
            publisher.verify_descriptor_edit(previous, replacement, edit)
        with self.assertRaisesRegex(AssertionError, 'one existing part'):
            publisher.verify_descriptor_edit(previous, replacement,
                {'gameplay': {}, 'partAppearances': {'missing': {}}})

    def test_rejects_concurrent_change_before_writing_any_asset(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'model').write_bytes(b'old')
            (root / 'index').write_bytes(b'concurrent')
            with self.assertRaisesRegex(AssertionError, 'Concurrent edit'):
                publisher.install(root, {'model': b'old', 'index': b'old index'},
                                  {'model': b'new', 'index': b'new index'})
            self.assertEqual((root / 'model').read_bytes(), b'old')
            self.assertEqual((root / 'index').read_bytes(), b'concurrent')

    def test_failure_restores_existing_files_and_removes_new_receipts(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'model').write_bytes(b'old')
            (root / 'index').write_bytes(b'old index')
            write = publisher.atomic_write

            def fail_index(path, data):
                if path.name == 'index':
                    raise OSError('simulated index failure')
                write(path, data)

            with patch.object(publisher, 'atomic_write', side_effect=fail_index):
                with self.assertRaisesRegex(OSError, 'simulated'):
                    publisher.install(root, {'model': b'old', 'receipt': None, 'index': b'old index'},
                                      {'model': b'new', 'receipt': b'receipt', 'index': b'new index'})
            self.assertEqual((root / 'model').read_bytes(), b'old')
            self.assertEqual((root / 'index').read_bytes(), b'old index')
            self.assertFalse((root / 'receipt').exists())


if __name__ == '__main__':
    unittest.main()
