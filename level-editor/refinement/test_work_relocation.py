import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from work_relocation import RelocationMap, discover_repo_root, file_sha256


class RelocationTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        (self.root / 'AGENTS.md').write_text('instructions')
        (self.root / 'Cargo.toml').write_text('[workspace]')
        (self.root / 'level-editor').mkdir()
        self.manifest = {'version': 1, 'path_mappings': [
            {'old_prefix': 'work/old', 'new_prefix': 'work/active/map'}],
            'historical_files': [], 'operational_files': []}

    def put(self, path, content):
        p = self.root / path
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(content)
        return file_sha256(p)

    def mapping(self):
        return RelocationMap(self.manifest, self.root)

    def test_root_discovery_survives_added_depth(self):
        p = self.root / 'level-editor/work/active/map/nested'
        p.mkdir(parents=True)
        self.assertEqual(discover_repo_root(p), self.root)
        self.assertRaises(ValueError, discover_repo_root, self.root.parent)

    def test_longest_prefix_and_no_substring_replacement(self):
        self.manifest['path_mappings'].append(
            {'old_prefix': 'work/old/retained', 'new_prefix': 'work/final/map'})
        m = self.mapping()
        self.assertEqual(m.translate_path('work/old/retained/a'), 'work/final/map/a')
        self.assertEqual(m.translate_path('work/older/a'), 'work/older/a')
        self.assertEqual(m.translate_path('run work/old/a'), 'run work/old/a')
        self.assertRaises(ValueError, m.translate_path, 'work/old/../../escape')

    def test_moved_frozen_pin_still_requires_identical_bytes(self):
        h = self.put('work/active/map/a', b'approved')
        m = self.mapping()
        self.assertEqual(m.translate_json({'path': 'work/old/a', 'sha256': h}),
                         {'path': 'work/active/map/a', 'sha256': h})
        self.put('work/active/map/a', b'edited')
        self.assertRaises(ValueError, m.translate_json, {'path': 'work/old/a', 'sha256': h})

    def test_missing_new_target_does_not_fall_back_to_old_file(self):
        h = self.put('work/old/a', b'legacy remains')
        self.assertRaises(ValueError, self.mapping().translate_json,
                          {'model': 'work/old/a', 'model_sha256': h})

    def test_explicit_historical_read_never_edits_evidence(self):
        raw = b'{"path":"work/old/a"}'
        h = self.put('work/evidence/record.json', raw)
        self.manifest['historical_files'].append(
            {'old_path': 'work/old/record.json', 'sha256': h,
             'preserved_path': 'work/evidence/record.json'})
        m = self.mapping()
        self.assertEqual(m.read_historical_json('work/old/record.json', h)['path'], 'work/old/a')
        self.assertEqual(m.read_historical_json('work/old/record.json', h, translate=True)['path'],
                         'work/active/map/a')
        self.assertEqual((self.root / 'work/evidence/record.json').read_bytes(), raw)
        self.assertRaises(ValueError, m.resolve_historical, 'work/old/missing', h)

    def test_explicit_semantic_copy_rebinds_parent_and_dictionary_hashes(self):
        old = self.put('work/evidence/child.json', b'{"path":"old"}')
        new = self.put('work/active/map/child.json', b'{"path":"new"}')
        self.manifest['historical_files'].append({'old_path': 'work/old/child.json',
            'sha256': old, 'preserved_path': 'work/evidence/child.json'})
        self.manifest['operational_files'].append({'old_path': 'work/old/child.json',
            'old_sha256': old, 'new_path': 'work/active/map/child.json', 'new_sha256': new})
        m = self.mapping()
        value = {'nested': [{'model_path': 'work/old/child.json', 'model_sha256': old}],
                 'pins': {'work/old/child.json': old}}
        output = m.translate_json(value)
        self.assertEqual(output['nested'][0]['model_sha256'], new)
        self.assertEqual(output['pins'], {'work/active/map/child.json': new})
        self.put('work/evidence/child.json', b'corrupted approval')
        self.assertRaises(ValueError, m.verify)

    def test_nonstandard_hash_fields_require_explicit_pair(self):
        old = self.put('work/evidence/child', b'old')
        new = self.put('work/active/map/child', b'new')
        self.manifest['historical_files'].append({'old_path': 'work/old/child',
            'sha256': old, 'preserved_path': 'work/evidence/child'})
        self.manifest['operational_files'].append({'old_path': 'work/old/child',
            'old_sha256': old, 'new_path': 'work/active/map/child', 'new_sha256': new})
        value = {'source_model': 'work/old/child', 'model_sha256': old}
        self.assertRaises(ValueError, self.mapping().translate_json, value)
        self.manifest['pin_pairs'] = [['source_model', 'model_sha256']]
        self.assertEqual(self.mapping().translate_json(value),
            {'source_model': 'work/active/map/child', 'model_sha256': new})

    def test_compatibility_symlink_is_rejected_in_parent_directory(self):
        h = self.put('elsewhere/a', b'data')
        (self.root / 'work/active').mkdir(parents=True)
        (self.root / 'work/active/map').symlink_to(self.root / 'elsewhere', target_is_directory=True)
        self.assertRaises(ValueError, self.mapping().translate_json,
                          {'file': 'work/old/a', 'sha256': h})

    def test_key_collision_is_not_silent(self):
        self.assertRaises(ValueError, self.mapping().translate_json,
                          {'work/old/a': 'first', 'work/active/map/a': 'second'})

    def test_cli_creates_new_manifest_receipt_and_refuses_overwrite(self):
        old = self.root / 'historical.json'
        old.write_text('{"path":"work/old/item"}')
        original = old.read_bytes()
        map_path = self.root / 'mapping.json'
        map_path.write_text(json.dumps(self.manifest))
        target = self.root / 'operational.json'
        command = [sys.executable, str(Path(__file__).with_name('work_relocation.py')),
                   'translate', str(map_path), str(old), str(target),
                   '--input-sha256', hashlib.sha256(original).hexdigest()]
        first = subprocess.run(command, capture_output=True, text=True)
        self.assertEqual(first.returncode, 0, first.stderr)
        self.assertEqual(json.loads(target.read_text())['path'], 'work/active/map/item')
        receipt = json.loads(target.with_name('operational.json.relocation.json').read_text())
        self.assertEqual(receipt['operational_sha256'], file_sha256(target))
        self.assertEqual(old.read_bytes(), original)
        self.assertNotEqual(subprocess.run(command, capture_output=True).returncode, 0)


if __name__ == '__main__':
    unittest.main()
