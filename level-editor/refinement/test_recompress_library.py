import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from recompress_library import recompress
from test_lossy_assets import glb, UNLIT


class RecompressLibraryTest(unittest.TestCase):
    def test_apply_preserves_source_and_binds_receipt_with_backups(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            root = base / 'library'
            asset = root / 'house'
            asset.mkdir(parents=True)
            source = glb(UNLIT)
            digest = hashlib.sha256(source).hexdigest()
            (asset / 'model.glb').write_bytes(source)
            (asset / 'lossy.glb').write_bytes(source)
            (asset / 'asset.json').write_text(json.dumps({'id': 'house', 'name': 'House',
                'source_map': 'Derby', 'model': 'model.glb'}))
            receipt = {'source': digest, 'output': digest, 'settings': {'quality': 80}}
            (asset / 'lossy.glb.receipt.json').write_text(json.dumps(receipt))
            (root / 'index.json').write_text('{"version":1,"assets":[]}')
            report = recompress(root, base / 'work', apply=True)
            self.assertTrue(report['applied'])
            self.assertEqual((asset / 'model.glb').read_bytes(), source)
            self.assertLessEqual((asset / 'lossy.glb').stat().st_size, len(source))
            result = json.loads((asset / 'lossy.glb.receipt.json').read_text())
            self.assertEqual(result['source'], digest)
            self.assertEqual(result['settings']['quality'], 80)
            self.assertEqual(result['settings']['geometry_compression'], 'meshopt-v1-if-smaller')
            self.assertEqual(result['output'], hashlib.sha256((asset / 'lossy.glb').read_bytes()).hexdigest())
            self.assertEqual(json.loads((base / 'work/backup/house/lossy.glb.receipt.json').read_text()), receipt)


if __name__ == '__main__':
    unittest.main()
