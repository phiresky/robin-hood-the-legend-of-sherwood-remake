import copy
import unittest
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

if __name__ == '__main__':
    unittest.main()
