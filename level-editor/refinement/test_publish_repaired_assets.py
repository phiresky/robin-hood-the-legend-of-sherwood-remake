"""Publication transactions reject stale files and restore partially written assets."""
import importlib.util
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
