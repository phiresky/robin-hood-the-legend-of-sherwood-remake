"""The catalog is a disposable, validated projection of asset directories."""
import hashlib
import json
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch
from asset_index import generate_asset_index, write_asset_index


class AssetIndexTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.asset = self.root/'derby/house'; self.asset.mkdir(parents=True)
        self.descriptor = {'id': 'house', 'name': 'House', 'source_map': 'Derby', 'model': 'model.glb',
                           'model_scene': 'default', 'tags': ['building'], 'asset_type': 'house'}
        (self.asset/'asset.json').write_text(json.dumps(self.descriptor))
        (self.asset/'model.glb').write_bytes(b'original')
        (self.asset/'lossy.glb').write_bytes(b'optimized')
        self.receipt = {key: hashlib.sha256((self.asset/name).read_bytes()).hexdigest()
                        for key, name in [('source', 'model.glb'), ('output', 'lossy.glb')]}
        (self.asset/'lossy.glb.receipt.json').write_text(json.dumps(self.receipt))
        write_asset_index(self.root)
        self.previous = (self.root/'index.json').read_bytes()

    def reject(self, pattern):
        with self.assertRaisesRegex(ValueError, pattern):
            write_asset_index(self.root)
        self.assertEqual((self.root/'index.json').read_bytes(), self.previous)
        self.assertEqual(list(self.root.glob('.index.json-*.tmp')), [])

    def test_generation_never_reads_previous_index_and_is_deterministic(self):
        (self.root/'index.json').write_bytes(b'not even JSON')
        write_asset_index(self.root)
        self.assertEqual((self.root/'index.json').read_bytes(), self.previous)
        (self.root/'index.json').unlink()
        index = write_asset_index(self.root)
        self.assertEqual(index['assets'][0]['tags'], ['building'])
        self.assertEqual(index['assets'][0]['model_scene'], 'default')
        self.assertEqual(index['assets'][0]['asset_type'], 'house')
        self.assertEqual(index['assets'][0]['descriptor_sha256'],
                         hashlib.sha256((self.asset/'asset.json').read_bytes()).hexdigest())
        self.assertEqual((self.root/'index.json').read_bytes(), self.previous)

    def test_editor_projection_excludes_reconstruction_evidence(self):
        descriptor = {**self.descriptor, 'version': 1, 'kind': 'projection-mapped-asset',
                      'parts': [{'node': 'building-000', 'name': 'Wall', 'source_obstacle': 0,
                                 'obstacle_local_game': {'points': []},
                                 'reprojection_source_path': '/private/review.png'}],
                      'components': [{'reprojection_source_path': '/private/review.png'}]}
        (self.asset/'asset.json').write_text(json.dumps(descriptor))
        editor = write_asset_index(self.root)['assets'][0]['editor']
        self.assertEqual(editor['parts'][0]['source_obstacle'], 0)
        self.assertNotIn('reprojection_source_path', editor['parts'][0])
        self.assertNotIn('components', editor)

    def test_directory_addition_removal_and_descriptor_edits_change_catalog(self):
        other = self.root/'another'; shutil.copytree(self.asset, other)
        descriptor = {**self.descriptor, 'id': 'another', 'name': 'Renamed'}
        (other/'asset.json').write_text(json.dumps(descriptor))
        index = write_asset_index(self.root)
        self.assertEqual([e['id'] for e in index['assets']], ['another', 'house'])
        self.assertEqual(index['assets'][0]['name'], 'Renamed')
        shutil.rmtree(self.asset)
        index = write_asset_index(self.root)
        self.assertEqual([e['id'] for e in index['assets']], ['another'])

    def test_nonrendering_gameplay_frames_survive_catalog_and_variant_projection(self):
        part = {'node': 'scenery-emitter', 'name': 'Ambient region',
                'scenery': True, 'gameplay_only': True}
        descriptor = {**self.descriptor, 'parts': [part],
                      'gameplay': {'version': 1, 'collision': 'none', 'surfaces': [],
                                   'doors': [], 'sounds': []},
                      'state_variants': {'initial': {'name': 'Initial', 'model': 'model.glb',
                                                     'parts': [part]}}}
        (self.asset/'asset.json').write_text(json.dumps(descriptor))
        editor = write_asset_index(self.root)['assets'][0]['editor']
        self.assertEqual(editor['parts'], [part])
        self.assertEqual(editor['state_variants']['initial']['parts'], [part])
        self.assertEqual(editor['gameplay'], descriptor['gameplay'])

    def test_source_change_rejects_even_with_a_missing_cached_index_entry(self):
        (self.asset/'model.glb').write_bytes(b'republished')
        self.reject('house: lossy receipt does not bind the current model')

    def test_gameplay_collision_and_join_metadata_survive_all_catalog_views(self):
        part = {'node': 'building-000', 'name': 'Visual shell', 'source_obstacle': 0,
                'appearance': {'show': ['activate']},
                'obstacle_local_game': {'points': []}, 'collision': 'none',
                'sight_join_edges': [[[0, 0, 0], [10, 0, 0]]],
                'sight_join_caps': ['top', 'bottom']}
        variant = {'name': 'Alternate', 'model': 'model.glb', 'parts': [part]}
        descriptor = {**self.descriptor, 'parts': [part],
                      'state_variants': {'applied': variant},
                      'standalone_variants': {'initial': variant},
                      'gameplay': {'version': 1, 'collision': 'parts', 'surfaces': [],
                                   'doors': [], 'draft': {'issues': ['Mask recovery incomplete.']}}}
        (self.asset/'asset.json').write_text(json.dumps(descriptor))
        editor = write_asset_index(self.root)['assets'][0]['editor']
        self.assertEqual(editor['parts'], [part])
        self.assertEqual(editor['state_variants']['applied']['parts'], [part])
        self.assertEqual(editor['standalone_variants']['initial']['parts'], [part])
        self.assertEqual(editor['gameplay'], descriptor['gameplay'])

    def test_corrupt_output_is_rejected(self):
        (self.asset/'lossy.glb').write_bytes(b'corrupt')
        self.reject('house: lossy model bytes differ')

    def test_missing_and_malformed_receipts_are_rejected(self):
        receipt = self.asset/'lossy.glb.receipt.json'
        for raw in ['[]', '{', '{}', '{"source": 1, "output": null}']:
            with self.subTest(raw=raw):
                receipt.write_text(raw)
                self.reject('house:')
        receipt.unlink()
        self.reject('house: lossy model or receipt missing')

    def test_orphan_receipt_and_missing_source_are_errors(self):
        (self.asset/'lossy.glb').unlink()
        self.reject('house: lossy model or receipt missing')
        (self.asset/'model.glb').unlink()
        self.reject('source model missing')

    def test_preflight_discovers_staged_descriptors_and_prospective_removals(self):
        stage = self.root/'.stage'; shutil.copytree(self.asset, stage)
        descriptor = {**self.descriptor, 'id': 'new', 'name': 'New'}
        (stage/'asset.json').write_text(json.dumps(descriptor))
        files = {'new/'+p.name: p for p in stage.iterdir()}
        files['derby/house/asset.json'] = None
        index = generate_asset_index(self.root, files=files)
        self.assertEqual([e['id'] for e in index['assets']], ['new'])
        (stage/'model.glb').write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'new: lossy receipt does not bind'):
            generate_asset_index(self.root, files=files)
        self.assertEqual((self.root/'index.json').read_bytes(), self.previous)

    def test_hidden_backups_blobs_and_symlink_directories_are_not_assets(self):
        for name in ('.stage', 'backups', 'backup', 'blobs'):
            shutil.copytree(self.asset, self.root/name/'duplicate')
        (self.root/'linked').symlink_to(self.asset, target_is_directory=True)
        self.assertEqual(generate_asset_index(self.root)['assets'], json.loads(self.previous)['assets'])

    def test_no_derivative_is_required_and_previews_are_discovered(self):
        (self.asset/'lossy.glb').unlink(); (self.asset/'lossy.glb.receipt.json').unlink()
        (self.asset/'preview.glb').write_bytes(b'preview')
        entry = write_asset_index(self.root)['assets'][0]
        self.assertNotIn('lossy_model', entry)
        self.assertEqual(entry['preview_model'], 'derby/house/preview.glb')

    def test_failed_replace_preserves_index_and_cleans_temporary(self):
        with patch('asset_index.os.replace', side_effect=OSError('injected')):
            with self.assertRaisesRegex(OSError, 'injected'): write_asset_index(self.root)
        self.assertEqual((self.root/'index.json').read_bytes(), self.previous)
        self.assertEqual(list(self.root.glob('.index.json-*.tmp')), [])

    def test_duplicate_ids_and_unsafe_paths_are_rejected(self):
        other = self.root/'another'; shutil.copytree(self.asset, other)
        self.reject('Duplicate asset index ID')
        shutil.rmtree(other)
        for value in ['../escape.glb', '/absolute.glb', 'a//b.glb']:
            (self.asset/'asset.json').write_text(json.dumps({**self.descriptor, 'model': value}))
            self.reject('Unsafe asset index path')

    def test_missing_descriptor_metadata_is_not_filled_from_old_index(self):
        for field in ('id', 'name', 'source_map', 'model'):
            descriptor = dict(self.descriptor); descriptor.pop(field)
            (self.asset/'asset.json').write_text(json.dumps(descriptor))
            self.reject('descriptor requires ' + field)


if __name__ == '__main__': unittest.main()
