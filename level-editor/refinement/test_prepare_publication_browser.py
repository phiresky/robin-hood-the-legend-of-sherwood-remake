import hashlib
import json
import os
from pathlib import Path
import struct
import tempfile
import unittest
from prepare_publication_browser import prepare, preserved_ungrouped
from scene_manifest import import_document


def stage_fixture():
    """A one-group York stage and empty live library in the current directory."""
    stage = Path('stage'); stage.mkdir(); (stage/'assets').mkdir()
    library = Path('level-editor/library')
    (library/'scenes').mkdir(parents=True); (library/'3d-assets').mkdir()
    for path in [stage/'assets/index.json', library/'3d-assets/index.json']:
        path.write_text('{"assets":[]}')
    (library/'scenes/york-volumes.scene.json').write_text('{}')
    model = {'asset': {'version': '2.0'}, 'scene': 0, 'scenes': [{'nodes': [0]}], 'nodes': [{'name':'map','children':[1]}, {'name':'House','extras':{'asset_group':'york-house'},'children':[2]}, {'name':'building-000'}]}
    encoded = json.dumps(model).encode(); encoded += b' ' * (-len(encoded) % 4)
    (stage/'york.scene.glb').write_bytes(struct.pack('<III',0x46546c67,2,20+len(encoded))+struct.pack('<II',len(encoded),0x4e4f534a)+encoded)
    document = {'size':[100,200], 'provenance':{}, 'groups':[{'id':'york-house'}], 'objects':[{'node':'building-000','group':'york-house'}]}
    document, _ = import_document(stage/'york.scene.glb', stage/'map-assets', document)
    (stage/'york.rhlos-map.json').write_text(json.dumps(document))
    Path('document.json').write_text(json.dumps(document))
    return stage, library, document


def write_asset(root, model_bytes):
    """york-house descriptor, model and a lossy derivative whose receipt binds that model."""
    folder = root/'york/york-house'; folder.mkdir(parents=True)
    (folder/'asset.json').write_text(json.dumps({'version': 1, 'kind': 'projection-mapped-asset', 'id': 'york-house',
                                                 'name': 'House', 'source_map': 'York', 'model': 'model.glb'}))
    (folder/'model.glb').write_bytes(model_bytes)
    (folder/'lossy.glb').write_bytes(b'lossy ' + model_bytes)
    digest = lambda data: hashlib.sha256(data).hexdigest()
    (folder/'lossy.glb.receipt.json').write_text(json.dumps({'source': digest(model_bytes),
                                                             'output': digest(b'lossy ' + model_bytes)}))
    return folder


class FirstPublicationTest(unittest.TestCase):
    def test_ungrouped_placements_preserve_pose_and_exact_asset_pins(self):
        from copy import deepcopy
        previous = {'objects': [{'id': 'ambient', 'node': 'asset:sound:emitter',
                                 'transform': {'dx': 4, 'dy': 5}}],
                    'assetSources': [{'id': 'sound', 'model_sha256': 'original'}]}
        self.assertEqual(preserved_ungrouped(deepcopy(previous), previous), 1)
        changed = deepcopy(previous); changed['objects'][0]['transform']['dx'] = 7
        with self.assertRaisesRegex(ValueError, 'placements changed'):
            preserved_ungrouped(changed, previous)
        changed = deepcopy(previous); changed['objects'] = []
        with self.assertRaisesRegex(ValueError, 'placements changed'):
            preserved_ungrouped(changed, previous)
        changed = deepcopy(previous); changed['assetSources'][0]['model_sha256'] = 'replaced'
        with self.assertRaisesRegex(ValueError, 'asset pins changed'):
            preserved_ungrouped(changed, previous)

    def test_explicit_document_is_staged_without_creating_live_document(self):
        previous = Path.cwd()
        with tempfile.TemporaryDirectory() as temporary:
            try:
                os.chdir(temporary)
                stage, library, document = stage_fixture()
                Path('scope.json').write_text('{"asset_ids":[],"already_published":[]}')
                result = prepare(stage,'scope.json','audit/config.json',map_name='york',document_path='document.json')
                self.assertEqual(result['groups'],1)
                self.assertFalse((library/'scenes/york.rhlos-map.json').exists())
                self.assertTrue((stage/'york.rhlos-map.json').exists())
                bad = dict(document,objects=[{'node':'building-000','group':'wrong'}]); Path('bad.json').write_text(json.dumps(bad))
                with self.assertRaisesRegex(ValueError,'ownership differs'):
                    prepare(stage,'scope.json','audit/other.json',map_name='york',document_path='bad.json')
                with self.assertRaisesRegex(ValueError,'cannot replace live'):
                    prepare(stage,'scope.json','audit/live.json',map_name='york',document_path='document.json',live=True)
            finally:
                os.chdir(previous)

    def test_staged_catalog_derivatives_replace_live_ones(self):
        # A changed asset keeps live lossy files bound to the old model; the audit must use the
        # staged catalog's refreshed derivatives, not the raw standalone export plus live files.
        previous = Path.cwd()
        with tempfile.TemporaryDirectory() as temporary:
            try:
                os.chdir(temporary)
                stage, library, document = stage_fixture()
                write_asset(library/'3d-assets', b'old model')
                staged = write_asset(stage/'map-assets/3d-assets', b'new model')
                raw = stage/'assets/york/york-house'; raw.mkdir(parents=True)
                for name in ('asset.json', 'model.glb'):
                    (raw/name).write_bytes((staged/name).read_bytes())
                Path('scope.json').write_text('{"asset_ids":["york-house"],"already_published":[]}')
                prepare(stage, 'scope.json', 'audit/config.json', map_name='york', document_path='document.json')
                files = {entry['path']: entry for entry in json.loads(Path('audit/config.json').read_text())['files']}
                for name in ('model.glb', 'lossy.glb'):
                    entry = files['3d-assets/york/york-house/' + name]
                    self.assertEqual(entry['url'], '/@fs/' + str((staged/name).resolve()))
            finally:
                os.chdir(previous)

    def test_staged_original_does_not_inherit_live_derivative(self):
        previous = Path.cwd()
        with tempfile.TemporaryDirectory() as temporary:
            try:
                os.chdir(temporary)
                stage, library, document = stage_fixture()
                write_asset(library/'3d-assets', b'old model')
                staged = write_asset(stage/'map-assets/3d-assets', b'new model')
                (staged/'lossy.glb').unlink()
                (staged/'lossy.glb.receipt.json').unlink()
                Path('scope.json').write_text('{"asset_ids":["york-house"],"already_published":[]}')
                prepare(stage, 'scope.json', 'audit/config.json', map_name='york', document_path='document.json')
                index = json.loads(Path('audit/private-index.json').read_text())
                self.assertNotIn('lossy_model', index['assets'][0])
            finally:
                os.chdir(previous)

if __name__ == '__main__':
    unittest.main()


class BoundPatchesTest(unittest.TestCase):
    def test_placements_bind_asset_local_appearances(self):
        from prepare_publication_browser import bound_patches
        nodes = [{'extras': {'reveal_hide_when_applied': ['appearance-1']}},
                 {'extras': {'reveal_show_when_applied': ['appearance-1', 'appearance-2']}},
                 {'extras': {'reveal_material_patch': 'patch-legacy'}}]
        document = {'groups': [{'id': 'hall', 'patches': {'hall': {'appearance-1': 'patch-010', 'appearance-2': 'patch-011'}}}],
                    'objects': [{'node': 'asset:gate:x', 'patches': {'gate': {'appearance-1': 'patch-003'}}}]}
        self.assertEqual(bound_patches(nodes, document), {'patch-010', 'patch-011', 'patch-003', 'patch-legacy'})
        with self.assertRaisesRegex(ValueError, 'appearance-2'):
            bound_patches(nodes, {'groups': [{'id': 'hall', 'patches': {'hall': {'appearance-1': 'patch-010'}}}]})
