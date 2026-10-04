"""Small filesystem fixtures exercise static endpoint promotion without Blender."""
import json
import struct
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import promote_staged_publication as promotion
from scene_manifest import import_document


def map_fixture(stage, name):
    model = {'asset': {'version': '2.0'}, 'scene': 0, 'scenes': [{'nodes': [0]}],
             'nodes': [{'name': 'map', 'children': [1]}, {'name': 'buildings', 'children': [2]}, {'name': 'building-000'}]}
    data = json.dumps(model).encode(); data += b' ' * (-len(data) % 4)
    (stage / f'{name}.scene.glb').write_bytes(struct.pack('<III', 0x46546c67, 2, 20+len(data)) + struct.pack('<II',len(data),0x4e4f534a) + data)
    document, _ = import_document(stage / f'{name}.scene.glb', stage / 'map-assets', {'map': name})
    (stage / f'{name}.rhlos-map.json').write_text(json.dumps(document))


class PromotionTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        root = Path(self.temporary.name)
        self.stage, self.library = root / 'stage', root / 'library'
        self.main = root / 'main.blend'
        self.stage.mkdir()
        (self.stage / 'assets/bridge').mkdir(parents=True)
        (self.library / '3d-assets/bridge').mkdir(parents=True)
        (self.library / 'scenes').mkdir()
        self.entry = {'id': 'bridge', 'descriptor': 'bridge/asset.json', 'model': 'bridge/raised.glb'}
        self.descriptor = {'id': 'bridge', 'name': 'Bridge', 'source_map': 'Derby', 'model': 'raised.glb'}
        self.write_json(self.stage / 'assets/index.json', {'assets': [self.entry]})
        self.write_json(self.library / '3d-assets/index.json', {'assets': []})
        for name in ('asset-verification', 'handoff-verification', 'browser-result'):
            self.write_json(self.stage / f'{name}.json', {'status': 'PASS'})
        for name in ('worker.blend', 'derby.scene.glb', 'derby.rhlos-map.json', 'assets/bridge/raised.glb', 'assets/bridge/lowered.glb'):
            (self.stage / name).write_bytes(('new:' + name).encode())
        map_fixture(self.stage, 'derby')
        self.main.write_bytes(b'old blend')
        self.write_json(self.library / 'scenes/derby-volumes.scene.json', {'protected': True})

    @staticmethod
    def write_json(path, value):
        path.write_text(json.dumps(value))

    def variants(self):
        self.descriptor['state_variants'] = {
            'initial': {'name': 'Raised', 'model': 'raised.glb'},
            'applied': {'name': 'Lowered', 'model': 'lowered.glb'},
        }

    def prepare(self):
        self.write_json(self.stage / 'assets/bridge/asset.json', self.descriptor)
        promotion.prepare(self.stage, self.library, self.main, 'derby')
        return json.loads((self.stage / 'promotion.json').read_text())

    def test_scene_import_publishes_manifest_and_immutable_assets(self):
        manifest = self.prepare()
        self.assertEqual(len(manifest['files']), 6)
        promotion.apply(self.stage / 'promotion.json')
        self.assertFalse((self.library / '3d-assets/bridge/lowered.glb').exists())

    def stage_derivatives(self, receipt=None):
        """Staged lossy model and preview for the bridge, with chained receipts."""
        import hashlib
        digest = lambda path: hashlib.sha256(path.read_bytes()).hexdigest()
        root = self.stage / 'assets'
        (root / 'bridge/lossy.glb').write_bytes(b'lossy')
        self.write_json(root / 'bridge/lossy.glb.receipt.json', receipt or
                        {'source': digest(root / 'bridge/raised.glb'), 'output': digest(root / 'bridge/lossy.glb')})
        (root / 'bridge/preview.glb').write_bytes(b'preview')
        self.write_json(root / 'bridge/preview.glb.receipt.json', {'source': digest(root / 'bridge/lossy.glb'),
                        'source_model': 'bridge/lossy.glb', 'output': digest(root / 'bridge/preview.glb')})
        self.entry.update(lossy_model='bridge/lossy.glb', preview_model='bridge/preview.glb')
        self.write_json(root / 'index.json', {'assets': [self.entry]})

    def test_lossy_models_and_previews_are_promoted_and_kept_in_the_index(self):
        self.stage_derivatives()
        manifest = self.prepare()
        assets = (self.library / '3d-assets').resolve()
        targets = {Path(item['target']).relative_to(assets).as_posix() for item in manifest['files']
                   if Path(item['target']).is_relative_to(assets)}
        self.assertLessEqual({'bridge/lossy.glb', 'bridge/lossy.glb.receipt.json', 'bridge/preview.glb',
                              'bridge/preview.glb.receipt.json'}, targets)
        promotion.apply(self.stage / 'promotion.json')
        live = {e['id']: e for e in json.loads((self.library / '3d-assets/index.json').read_text())['assets']}
        self.assertEqual((live['bridge']['lossy_model'], live['bridge']['preview_model']),
                         ('bridge/lossy.glb', 'bridge/preview.glb'))
        self.assertEqual((self.library / '3d-assets/bridge/lossy.glb').read_bytes(), b'lossy')

    def test_stale_lossy_receipts_block_preparation(self):
        self.stage_derivatives(receipt={'source': 'a' * 64, 'output': 'b' * 64})
        self.write_json(self.stage / 'assets/bridge/asset.json', self.descriptor)
        with self.assertRaisesRegex(ValueError, 'Stale staged derivatives'):
            promotion.prepare(self.stage, self.library, self.main, 'derby')
        self.assertFalse((self.stage / 'promotion.json').exists())

    def test_retired_derivatives_are_backed_up_removed_and_rolled_back_on_failure(self):
        for fail in (False, True):
            with self.subTest(fail=fail):
                # This fixture's standalone asset is promoted alongside the map.
                root = self.library / '3d-assets/bridge'
                old = {'lossy.glb': b'old lossy', 'lossy.glb.receipt.json': b'old receipt'}
                for name, data in old.items():
                    (root / name).write_bytes(data)
                manifest = self.prepare()
                removals = [item for item in manifest['files'] if item['source'] is None]
                self.assertEqual(len(removals), 2)
                copy = promotion.shutil.copy2
                def fail_document(source, target):
                    if fail and Path(source) == self.stage / 'derby.rhlos-map.json':
                        raise OSError('after derivative removal')
                    return copy(source, target)
                with patch.object(promotion.shutil, 'copy2', side_effect=fail_document):
                    if fail:
                        with self.assertRaisesRegex(OSError, 'after derivative removal'):
                            promotion.apply(self.stage / 'promotion.json')
                    else:
                        promotion.apply(self.stage / 'promotion.json')
                for item in removals:
                    self.assertEqual(Path(item['backup']).read_bytes(), old[Path(item['target']).name])
                    self.assertEqual(Path(item['target']).exists(), fail)
                # Keep the second attempt independent of the first one's backups.
                (self.stage / 'promotion.json').unlink()
                promotion.shutil.rmtree(self.stage / 'promotion-backup')

    def test_variants_deduplicate_default_and_copy_both_with_hashes(self):
        self.variants()
        manifest = self.prepare()
        self.assertEqual(len(manifest['files']), 7)
        self.assertEqual(len({item['target'] for item in manifest['files']}), 7)
        promotion.apply(self.stage / 'promotion.json')
        for item in manifest['files']:
            self.assertEqual(promotion.sha(Path(item['target'])), item['source_sha256'])
        self.assertEqual(json.loads((self.stage / 'promotion.json').read_text())['status'], 'APPLIED')

    def test_variant_hash_change_rejects_before_any_write(self):
        self.variants()
        self.prepare()
        (self.stage / 'assets/bridge/lowered.glb').write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'input/target changed'):
            promotion.apply(self.stage / 'promotion.json')
        self.assertEqual(self.main.read_bytes(), b'old blend')

    def test_latest_other_map_directories_are_discovered_and_index_is_backed_up(self):
        self.prepare()
        index=self.library/'3d-assets/index.json'
        latest={'assets':[{'id':'other-map-new','model':'new.glb'}], 'metadata':'retained'}
        self.write_json(index, latest)
        other = self.library/'3d-assets/other-map-new'; other.mkdir()
        (other/'model.glb').write_bytes(b'new asset')
        self.write_json(other/'asset.json', {'id':'other-map-new', 'name':'New', 'source_map':'York', 'model':'model.glb'})
        previous=index.read_bytes()
        promotion.apply(self.stage/'promotion.json')
        merged=json.loads(index.read_text())
        self.assertNotIn('metadata', merged)
        self.assertEqual({a['id'] for a in merged['assets']},{'bridge','other-map-new'})
        manifest=json.loads((self.stage/'promotion.json').read_text())
        self.assertEqual(manifest['files'][-1]['target'],str(index))
        self.assertEqual(Path(manifest['files'][-1]['backup']).read_bytes(),previous)

    def test_staged_index_is_disposable_and_does_not_control_publication(self):
        self.prepare()
        (self.stage/'assets/index.json').write_text('not valid JSON')
        promotion.apply(self.stage/'promotion.json')
        index = json.loads((self.library/'3d-assets/index.json').read_text())
        self.assertEqual([entry['id'] for entry in index['assets']], ['bridge'])


    def test_external_index_race_preserves_new_index_and_rolls_back_assets(self):
        self.prepare()
        index=self.library/'3d-assets/index.json'
        concurrent={'assets':[{'id':'concurrent-map'}]}
        copy=promotion.shutil.copy2
        def race(source,target):
            result=copy(source,target)
            if Path(source)==self.stage/'worker.blend':
                self.write_json(index,concurrent)
            return result
        with patch.object(promotion.shutil,'copy2',side_effect=race):
            with self.assertRaisesRegex(ValueError,'target changed before write'):
                promotion.apply(self.stage/'promotion.json')
        self.assertEqual(json.loads(index.read_text()),concurrent)
        self.assertEqual(self.main.read_bytes(),b'old blend')

    def test_standalone_endpoints_keep_covered_model_and_copy_both(self):
        (self.stage / 'assets/bridge/covered.glb').write_bytes(b'covered default')
        self.entry['model'] = 'bridge/covered.glb'
        self.descriptor['model'] = 'covered.glb'
        self.write_json(self.stage / 'assets/index.json', {'assets': [self.entry]})
        self.variants()
        self.descriptor['standalone_variants'] = self.descriptor.pop('state_variants')
        manifest = self.prepare()
        self.assertEqual(len(manifest['files']), 8)
        promotion.apply(self.stage / 'promotion.json')
        for name in ('covered.glb', 'raised.glb', 'lowered.glb'):
            self.assertEqual((self.library / '3d-assets/bridge' / name).read_bytes(),
                             (self.stage / 'assets/bridge' / name).read_bytes())

    def test_standalone_variants_reject_conflicting_metadata_and_unsafe_paths(self):
        self.variants()
        self.descriptor['standalone_variants'] = dict(self.descriptor['state_variants'])
        self.write_json(self.stage / 'assets/bridge/asset.json', self.descriptor)
        with self.assertRaisesRegex(ValueError, 'Conflicting'):
            promotion.asset_file_pairs(self.stage / 'assets', self.library / '3d-assets', self.entry)
        del self.descriptor['state_variants']
        self.descriptor['standalone_variants']['applied']['model'] = '../escape.glb'
        self.write_json(self.stage / 'assets/bridge/asset.json', self.descriptor)
        with self.assertRaisesRegex(ValueError, 'Unsafe'):
            promotion.asset_file_pairs(self.stage / 'assets', self.library / '3d-assets', self.entry)

    def test_late_variant_failure_rolls_back_old_and_new_files(self):
        self.variants()
        raised = self.library / '3d-assets/bridge/raised.glb'
        raised.write_bytes(b'old raised')
        manifest = self.prepare()
        copy = promotion.shutil.copy2

        def fail_variant(source, target):
            if Path(source) == self.stage / 'assets/bridge/lowered.glb':
                raise OSError('simulated variant copy failure')
            return copy(source, target)

        with patch.object(promotion.shutil, 'copy2', side_effect=fail_variant):
            with self.assertRaisesRegex(OSError, 'simulated'):
                promotion.apply(self.stage / 'promotion.json')
        for item in manifest['files']:
            self.assertEqual(promotion.sha(Path(item['target'])), item['previous_sha256'])

    def test_unsafe_variant_paths_and_symlinks_are_rejected(self):
        self.variants()
        for model in ('/outside.glb', '../outside.glb', 'nested/../../outside.glb', 'a\\b.glb', 'a//b.glb', ''):
            with self.subTest(model=model):
                self.descriptor['state_variants']['applied']['model'] = model
                self.write_json(self.stage / 'assets/bridge/asset.json', self.descriptor)
                with self.assertRaisesRegex(ValueError, 'Unsafe'):
                    promotion.asset_file_pairs(self.stage / 'assets', self.library / '3d-assets', self.entry)
        self.descriptor['state_variants']['applied']['model'] = 'escape.glb'
        self.write_json(self.stage / 'assets/bridge/asset.json', self.descriptor)
        (self.stage / 'assets/bridge/escape.glb').symlink_to(self.main)
        with self.assertRaisesRegex(ValueError, 'escapes'):
            promotion.asset_file_pairs(self.stage / 'assets', self.library / '3d-assets', self.entry)
        self.descriptor['state_variants']['applied']['model'] = 'lowered.glb'
        self.write_json(self.stage / 'assets/bridge/asset.json', self.descriptor)
        (self.library / '3d-assets/bridge/lowered.glb').symlink_to(self.main)
        with self.assertRaisesRegex(ValueError, 'escapes'):
            promotion.asset_file_pairs(self.stage / 'assets', self.library / '3d-assets', self.entry)

    def test_variant_metadata_rejects_unknown_endpoints(self):
        self.descriptor['state_variants'] = {'moving': {'name': 'Moving', 'model': 'lowered.glb'}}
        with self.assertRaisesRegex(ValueError, 'Invalid static'):
            self.prepare()

    def test_catalog_is_hash_guarded_and_rolled_back_with_models(self):
        self.variants()
        self.write_json(self.stage / 'assets/bridge/asset.json', self.descriptor)
        source, target = self.stage / 'catalog.json', self.library / 'catalog.json'
        self.write_json(source, {'map': 'Derby', 'groups': [{'id': 'bridge'}]})
        target.write_bytes(b'old catalog')
        promotion.prepare(self.stage, self.library, self.main, 'derby', source, target)
        manifest = json.loads((self.stage / 'promotion.json').read_text())
        self.assertEqual(len(manifest['files']), 8)
        copy = promotion.shutil.copy2

        def fail_variant(source_file, target_file):
            if Path(source_file) == self.stage / 'assets/bridge/lowered.glb':
                raise OSError('simulated variant failure')
            return copy(source_file, target_file)

        with patch.object(promotion.shutil, 'copy2', side_effect=fail_variant):
            with self.assertRaisesRegex(OSError, 'simulated'):
                promotion.apply(self.stage / 'promotion.json')
        self.assertEqual(target.read_bytes(), b'old catalog')
        self.assertTrue(any(Path(item['backup']).read_bytes() == b'old catalog'
                            for item in manifest['files'] if item['target'] == str(target)))

    def test_catalog_pair_and_map_must_match(self):
        source = self.stage / 'catalog.json'
        self.write_json(source, {'map': 'York', 'groups': []})
        with self.assertRaisesRegex(ValueError, 'together'):
            promotion.prepare(self.stage, self.library, self.main, 'derby', source)
        with self.assertRaisesRegex(ValueError, 'published map'):
            promotion.prepare(self.stage, self.library, self.main, 'derby', source, self.library / 'catalog.json')

    def test_catalog_promotes_with_matching_hash_and_backup(self):
        self.write_json(self.stage / 'assets/bridge/asset.json', self.descriptor)
        source, target = self.stage / 'catalog.json', self.library / 'catalog.json'
        self.write_json(source, {'map': 'Derby', 'groups': []})
        target.write_bytes(b'previous catalog')
        promotion.prepare(self.stage, self.library, self.main, 'derby', source, target)
        promotion.apply(self.stage / 'promotion.json')
        self.assertEqual(target.read_bytes(), source.read_bytes())
        manifest = json.loads((self.stage / 'promotion.json').read_text())
        record = next(item for item in manifest['files'] if item['target'] == str(target))
        self.assertEqual(record['source_sha256'], promotion.sha(target))
        self.assertEqual(Path(record['backup']).read_bytes(), b'previous catalog')


class FirstPublicationTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.stage = self.root / 'stage'
        self.library = self.root / 'library'
        self.main = self.root / 'main' / 'leicester.blend'
        for path in [self.stage / 'assets', self.library / '3d-assets', self.library / 'scenes']:
            path.mkdir(parents=True)
        for name in ['asset-verification.json', 'handoff-verification.json', 'browser-result.json']:
            (self.stage / name).write_text('{"status":"PASS"}')
        (self.stage / 'assets/index.json').write_text('{"assets":[]}')
        (self.library / '3d-assets/index.json').write_text('{"assets":[{"id":"other-map"}]}')
        other = self.library/'3d-assets/other-map'; other.mkdir()
        (other/'model.glb').write_bytes(b'other')
        (other/'asset.json').write_text(json.dumps({'id':'other-map', 'name':'Other', 'source_map':'York', 'model':'model.glb'}))
        for name in ['worker.blend', 'leicester.scene.glb', 'leicester.rhlos-map.json']:
            (self.stage / name).write_text('staged ' + name)
        (self.library / 'scenes/leicester-volumes.scene.glb').write_text('old map')
        (self.library / 'scenes/leicester-volumes.scene.json').write_text('protected document')
        map_fixture(self.stage, 'leicester')
        promotion.prepare(self.stage, self.library, self.main, 'leicester')

    def test_first_publication_creates_main_and_preserves_other_entries(self):
        promotion.apply(self.stage / 'promotion.json')
        self.assertEqual(self.main.read_text(), 'staged worker.blend')
        index = json.loads((self.library / '3d-assets/index.json').read_text())
        self.assertEqual([entry['id'] for entry in index['assets']], ['other-map'])
        self.assertEqual((self.library / 'scenes/leicester-volumes.scene.json').read_text(), 'protected document')
        report = json.loads((self.stage / 'promotion.json').read_text())
        self.assertEqual(report['status'], 'APPLIED')
        self.assertFalse(any(r['target'].endswith('-volumes.scene.glb') for r in report['files']))
        document = json.loads((self.library / 'scenes/leicester.rhlos-map.json').read_text())
        self.assertNotIn('glb', document)
        self.assertEqual(len(document['sceneAssets']), 1)

    def test_new_target_created_after_preparation_blocks_publication(self):
        self.main.parent.mkdir()
        self.main.write_text('concurrent change')
        with self.assertRaisesRegex(ValueError, 'Promotion input/target changed'):
            promotion.apply(self.stage / 'promotion.json')
        self.assertEqual((self.library / 'scenes/leicester-volumes.scene.glb').read_text(), 'old map')
        self.assertEqual(self.main.read_text(), 'concurrent change')


if __name__ == '__main__':
    unittest.main()
