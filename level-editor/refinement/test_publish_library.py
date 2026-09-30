import hashlib
import json
from pathlib import Path
import struct
import tempfile
import unittest
from unittest.mock import patch

from publish_library import stage_library, put_catalog, put_model
from cloudflare_publish import worker_config


def glb(document):
    raw = json.dumps(document).encode()
    raw += b' ' * (-len(raw) % 4)
    return struct.pack('<5I', 0x46546c67, 2, 20 + len(raw), len(raw), 0x4e4f534a) + raw


class PublishLibraryTest(unittest.TestCase):
    def test_large_model_parts_preserve_exact_runtime_bytes(self):
        data = bytes(range(256)) * 10
        payloads, index = {}, {}
        with patch('publish_library.MAX_FILE_BYTES', 1000):
            put_model('3d-assets/house/lossy.glb', data, index,
                      lambda path, value: payloads.__setitem__(path, value))
        model = index['model_shards']['3d-assets/house/lossy.glb']
        rebuilt = b''.join(payloads[part['path']] for part in model['parts'])
        self.assertEqual(rebuilt, data)
        self.assertEqual(model['bytes'], len(data))
        self.assertEqual(model['sha256'], hashlib.sha256(data).hexdigest())
        for part in model['parts']:
            self.assertLessEqual(len(payloads[part['path']]), 1000)
            self.assertEqual(part['sha256'], hashlib.sha256(payloads[part['path']]).hexdigest())

    def test_large_catalog_uses_content_addressed_batches(self):
        index = {'version': 1, 'assets': [{'id': str(i), 'editor': {'data': 'x' * 250}}
                                         for i in range(8)]}
        payloads = {}
        with patch('publish_library.MAX_FILE_BYTES', 1000):
            put_catalog(index, lambda path, data: payloads.__setitem__(path, data))
        manifest = json.loads(payloads['3d-assets/index.json'])
        restored = []
        for shard in manifest['asset_shards']:
            data = payloads[shard['path']]
            self.assertLessEqual(len(data), 1000)
            self.assertEqual(hashlib.sha256(data).hexdigest(), shard['sha256'])
            restored.extend(json.loads(data)['assets'])
        self.assertEqual(restored, index['assets'])

    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.library = self.root/'library'
        self.asset = self.library/'3d-assets/house'
        self.asset.mkdir(parents=True)
        (self.library/'scenes').mkdir()
        (self.library/'game-data').mkdir()
        (self.library/'game-data/index.json').write_text(json.dumps({'version': 1, 'files': []}))
        (self.asset/'asset.json').write_text(json.dumps(
            {'id': 'house', 'name': 'House', 'source_map': 'Derby', 'model': 'model.glb'}))
        (self.asset/'model.glb').write_bytes(b'original')
        self.lossy(glb({'asset': {'version': '2.0'}}))
        (self.asset/'private.txt').write_text('authoring only')

    def lossy(self, data):
        (self.asset/'lossy.glb').write_bytes(data)
        (self.asset/'lossy.glb.receipt.json').write_text(json.dumps({
            'source': hashlib.sha256(b'original').hexdigest(),
            'output': hashlib.sha256(data).hexdigest()}))

    def stage(self):
        return stage_library(self.library, self.root/'deploy')

    def test_allowlist_and_source_identity(self):
        report = self.stage()
        self.assertEqual(set(report['payloads']), {'3d-assets/house/lossy.glb',
            '3d-assets/index.json', 'scenes/index.json', 'game-data/index.json'})
        site = self.root/'deploy/site/editor/library'
        index = json.loads((site/'3d-assets/index.json').read_bytes())
        entry = index['assets'][0]
        self.assertEqual(entry['model_sha256'], hashlib.sha256(b'original').hexdigest())
        self.assertEqual(entry['preview_model'], entry['lossy_model'])
        self.assertEqual(entry['descriptor_sha256'], hashlib.sha256((self.asset/'asset.json').read_bytes()).hexdigest())
        self.assertFalse((site/'3d-assets/house/asset.json').exists())
        self.assertFalse((site/'3d-assets/house/model.glb').exists())
        config = json.loads((self.root/'deploy/wrangler.json').read_bytes())
        self.assertEqual(config['name'], 'robinhood-editor-library')
        self.assertEqual([r['pattern'] for r in config['routes']], [
            'robinhood.phiresky.xyz/editor/library', 'robinhood.phiresky.xyz/editor/library/*'])
        self.assertFalse(config['workers_dev'])

    def test_map_thumbnails_are_shipped_beside_maps(self):
        (self.library/'scenes/test.rhlos-map.json').write_text(json.dumps({'version': 1}))
        (self.library/'scenes/test.webp').write_bytes(b'thumbnail')
        (self.library/'scenes/unrelated.webp').write_bytes(b'not a map preview')
        report = self.stage()
        self.assertIn('scenes/test.webp', report['payloads'])
        self.assertNotIn('scenes/unrelated.webp', report['payloads'])
        self.assertEqual((self.root/'deploy/site/editor/library/scenes/test.webp').read_bytes(), b'thumbnail')

    def test_nonrendering_sound_frames_survive_publication(self):
        descriptor = json.loads((self.asset/'asset.json').read_bytes())
        descriptor['parts'] = [{'node': 'scenery-emitter', 'scenery': True,
                                'gameplay_only': True}]
        descriptor['gameplay'] = {'version': 1, 'collision': 'none',
            'surfaces': [], 'doors': [], 'sounds': [{'id': 'ambient',
            'node': 'scenery-emitter', 'sample': 54, 'active': True,
            'kind': 2, 'delay': [150, 500, 5], 'altitude': 1, 'ambiences': 255}]}
        (self.asset/'asset.json').write_text(json.dumps(descriptor))
        self.stage()
        index = json.loads((self.root/'deploy/site/editor/library/3d-assets/index.json').read_bytes())
        editor = index['assets'][0]['editor']
        self.assertEqual(editor['parts'], descriptor['parts'])
        self.assertEqual(editor['gameplay'], descriptor['gameplay'])

    def test_indexed_game_data_is_shipped(self):
        game_data = self.library/'game-data'
        files = {'Data/Levels/Mission.rhm.json': b'{"mission":1}',
                 'Data/Characters/Guard.rhs.d/atlas.webp': b'atlas'}
        for relative, data in files.items():
            path = game_data/relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
        (game_data/'notes.txt').write_text('not runtime data')
        (game_data/'index.json').write_text(json.dumps({'version': 1, 'files': list(files)}))
        report = self.stage()
        site = self.root/'deploy/site/editor/library/game-data'
        for relative, data in {**files, 'index.json': (game_data/'index.json').read_bytes()}.items():
            self.assertEqual((site/relative).read_bytes(), data)
            self.assertEqual(report['payloads']['game-data/'+relative], {
                'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()})
        self.assertFalse((site/'notes.txt').exists())

    def test_gameplay_only_frames_need_no_lossy_derivative(self):
        descriptor = json.loads((self.asset/'asset.json').read_bytes())
        descriptor['parts'] = [{'node': 'frame', 'scenery': True, 'gameplay_only': True}]
        (self.asset/'asset.json').write_text(json.dumps(descriptor))
        (self.asset/'lossy.glb').unlink()
        (self.asset/'lossy.glb.receipt.json').unlink()
        model = {'asset': {'version': '2.0'}, 'nodes': [{'name': 'frame',
                 'extras': {'scenery': True, 'gameplay_only': True}}],
                 'scenes': [{'nodes': [0]}], 'scene': 0}
        original = glb(model)
        (self.asset/'model.glb').write_bytes(original)
        report = self.stage()
        runtime = '3d-assets/house/model.runtime.glb'
        self.assertIn(runtime, report['payloads'])
        site = self.root/'deploy/site/editor/library'
        index = json.loads((site/'3d-assets/index.json').read_bytes())
        self.assertEqual(index['assets'][0]['lossy_model'], 'house/model.runtime.glb')
        self.assertEqual(index['assets'][0]['model_sha256'], hashlib.sha256(original).hexdigest())
        output = (site/runtime).read_bytes()
        size = struct.unpack_from('<I', output, 12)[0]
        self.assertEqual(json.loads(output[20:20+size]), model)
        self.assertFalse((site/'3d-assets/house/model.glb').exists())
        self.assertEqual((self.asset/'model.glb').read_bytes(), original)

    def test_visual_assets_still_require_lossy_models(self):
        (self.asset/'lossy.glb').unlink()
        (self.asset/'lossy.glb.receipt.json').unlink()
        with self.assertRaisesRegex(ValueError, 'current lossy GLB'):
            self.stage()

    def test_missing_game_data_index_rejected(self):
        (self.library/'game-data/index.json').unlink()
        with self.assertRaisesRegex(ValueError, 'run pnpm library:game-data'):
            self.stage()

    def test_invalid_game_data_index_rejected(self):
        (self.library/'game-data/index.json').write_text('{"version":2,"files":[]}')
        with self.assertRaisesRegex(ValueError, 'Invalid game data index'):
            self.stage()

    def test_missing_indexed_game_data_rejected(self):
        (self.library/'game-data/index.json').write_text(json.dumps({'version': 1, 'files': ['missing']}))
        with self.assertRaises(FileNotFoundError):
            self.stage()

    def test_game_data_path_escape_rejected(self):
        (self.library/'game-data/index.json').write_text(json.dumps({
            'version': 1, 'files': ['../3d-assets/house/model.glb']}))
        with self.assertRaisesRegex(ValueError, 'Unsafe runtime library path'):
            self.stage()

    def test_game_data_symlink_escape_rejected(self):
        (self.library/'game-data/secret').symlink_to(self.asset/'model.glb')
        (self.library/'game-data/index.json').write_text(json.dumps({'version': 1, 'files': ['secret']}))
        with self.assertRaisesRegex(ValueError, 'escapes its root'):
            self.stage()

    def test_stale_source_rejected(self):
        (self.asset/'model.glb').write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'receipt'):
            self.stage()

    def test_external_resources_rejected(self):
        self.lossy(glb({'images': [{'uri': 'private.png'}]}))
        with self.assertRaisesRegex(ValueError, 'external resource'):
            self.stage()

    def test_stale_map_pin_rejected(self):
        (self.library/'scenes/test.rhlos-map.json').write_text(json.dumps({
            'version': 1, 'sceneAssets': [{'model': '3d-assets/house/model.glb',
                                        'model_sha256': '0'*64}]}))
        with self.assertRaisesRegex(ValueError, 'no matching optimized'):
            self.stage()

    def test_editor_routes(self):
        config = worker_config('robinhood-editor', '/editor', 'auto-trailing-slash')
        self.assertEqual([r['pattern'] for r in config['routes']], [
            'robinhood.phiresky.xyz/editor', 'robinhood.phiresky.xyz/editor/*'])
        self.assertEqual(config['assets']['html_handling'], 'auto-trailing-slash')


if __name__ == '__main__':
    unittest.main()
