import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location(
    'deduplicate_gallery_history',
    Path(__file__).with_name('deduplicate_gallery_history.py'))
dedup = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dedup)


class HistoryDeduplication(unittest.TestCase):
    def test_preserves_paths_bytes_modes_and_is_repeatable(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            history = root / 'gallery' / 'history'
            history.mkdir(parents=True)
            for name, content in [('a', b'same'), ('b', b'same'),
                                  ('c', b'else'), ('d', b'same')]:
                (history / name).write_bytes(content)
            (history / 'd').chmod(0o600)
            expected = {p.name: p.read_bytes() for p in history.iterdir()}
            receipt = root / 'receipt.jsonl'
            dedup.run(history, receipt, True)
            self.assertEqual(expected, {p.name: p.read_bytes() for p in history.iterdir()})
            self.assertEqual((history / 'a').stat().st_ino, (history / 'b').stat().st_ino)
            self.assertNotEqual((history / 'a').stat().st_ino, (history / 'd').stat().st_ino)
            again = root / 'again.jsonl'
            dedup.run(history, again, True)
            self.assertEqual(json.loads(again.with_suffix('.summary.json').read_text())['replacements'], 0)

    def test_rejects_symlinks_before_changes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            history = root / 'gallery' / 'history'
            history.mkdir(parents=True)
            (root / 'outside').write_bytes(b'same')
            (history / 'link').symlink_to(root / 'outside')
            with self.assertRaises(ValueError):
                dedup.run(history, root / 'receipt.jsonl', True)
            self.assertTrue((history / 'link').is_symlink())

    def test_dry_run_does_not_share_inodes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            history = root / 'gallery' / 'history'
            history.mkdir(parents=True)
            for name in ['a', 'b']:
                (history / name).write_bytes(b'same')
            dedup.run(history, root / 'receipt.jsonl')
            self.assertNotEqual((history / 'a').stat().st_ino, (history / 'b').stat().st_ino)


if __name__ == '__main__':
    unittest.main()
