import io
import json
import unittest
from unittest.mock import patch

import ijson

from read_asset_index_entry import read_entry


class BoundedStream(io.BytesIO):
    def read(self, size=-1):
        if not 0 <= size <= 65536:
            raise AssertionError(f'Unbounded catalog read: {size}')
        return super().read(size)

    def readinto(self, buffer):
        if len(buffer) > 65536:
            raise AssertionError(f'Unbounded catalog readinto: {len(buffer)}')
        return super().readinto(buffer)


class ReadAssetIndexEntryTests(unittest.TestCase):
    def test_streams_past_large_embedded_descriptors_and_keeps_reference_pins(self):
        entry = {
            'id': 'é-test', 'descriptor': 'test/asset.json',
            'descriptor_sha256': 'a' * 64, 'model': 'test/model.glb',
            'model_scene': 'reviewed', 'editor': {'large': 'ignored'},
        }
        data = json.dumps({'version': 1, 'assets': [
            {'id': 'other', 'editor': {'escaped': '\\"[]{}' * 40000}}, entry,
        ]}, ensure_ascii=False).encode()
        with patch('pathlib.Path.open', return_value=BoundedStream(data)):
            result = read_entry('catalog.json', 'é-test')
        self.assertEqual(result, {k: v for k, v in entry.items() if k != 'editor'})

    def test_missing_asset_is_an_error(self):
        with patch('pathlib.Path.open', return_value=BoundedStream(b'{"assets":[]}')):
            with self.assertRaisesRegex(ValueError, 'absent.*missing'):
                read_entry('catalog.json', 'missing')

    def test_invalid_catalog_is_not_treated_as_a_missing_asset(self):
        with patch('pathlib.Path.open', return_value=BoundedStream(b'{"assets": [{')):
            with self.assertRaises(ijson.JSONError):
                read_entry('catalog.json', 'missing')


if __name__ == '__main__':
    unittest.main()
