"""Lossy derivative bookkeeping (receipts, index fields, library writes) without Blender."""
import hashlib
import json
from pathlib import Path
import struct
import sys
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import Mock, patch

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent / 'blender'))
import lossy_assets  # noqa: E402


def sha(data):
    return hashlib.sha256(data).hexdigest()


def glb(material, extra_materials=()):
    """One textured triangle; `material` is the glTF material JSON (extras are unreferenced)."""
    positions = struct.pack('<9f', 0, 0, 0, 1, 0, 0, 0, 1, 0)
    uvs = struct.pack('<6f', 0, 0, 1, 0, 0, 1)
    indices = struct.pack('<3H', 0, 1, 2) + b'\0\0'
    image = b'\x89PNG fake'
    body = positions + uvs + indices + image
    body += b'\0' * (-len(body) % 4)
    doc = {'asset': {'version': '2.0'}, 'scene': 0, 'scenes': [{'name': 'default', 'nodes': [0]}],
           'nodes': [{'name': 'part', 'mesh': 0}],
           'meshes': [{'primitives': [{'attributes': {'POSITION': 0, 'TEXCOORD_0': 1}, 'indices': 2, 'material': 0}]}],
           'materials': [material, *extra_materials], 'textures': [{'source': 0}], 'images': [{'bufferView': 3, 'mimeType': 'image/png'}],
           'accessors': [{'bufferView': 0, 'componentType': 5126, 'count': 3, 'type': 'VEC3', 'min': [0, 0, 0], 'max': [1, 1, 0]},
                         {'bufferView': 1, 'componentType': 5126, 'count': 3, 'type': 'VEC2'},
                         {'bufferView': 2, 'componentType': 5123, 'count': 3, 'type': 'SCALAR'}],
           'bufferViews': [{'buffer': 0, 'byteOffset': 0, 'byteLength': 36}, {'buffer': 0, 'byteOffset': 36, 'byteLength': 24},
                           {'buffer': 0, 'byteOffset': 60, 'byteLength': 6}, {'buffer': 0, 'byteOffset': 68, 'byteLength': len(image)}],
           'buffers': [{'byteLength': len(body)}]}
    chunk = json.dumps(doc).encode()
    chunk += b' ' * (-len(chunk) % 4)
    return (struct.pack('<4sII', b'glTF', 2, 12 + 8 + len(chunk) + 8 + len(body)) + struct.pack('<II', len(chunk), 0x4E4F534A)
            + chunk + struct.pack('<II', len(body), 0x004E4942) + body)


UNLIT = {'pbrMetallicRoughness': {'baseColorTexture': {'index': 0}}, 'extensions': {'KHR_materials_unlit': {}}}


class LossyAssetsTest(unittest.TestCase):
    def test_unsafe_box_pack_restarts_with_convex_without_mutating_settings(self):
        images, layers = Mock(), Mock()
        obj = SimpleNamespace(material_slots=[], data=SimpleNamespace(uv_layers=layers))
        args = SimpleNamespace(pack_shape='AABB')
        with patch.dict(sys.modules, {'bpy': SimpleNamespace(data=SimpleNamespace(images=images))}), \
                patch.object(lossy_assets, 'unwrap_square', side_effect=[lossy_assets.UnsafeAtlasError('collapsed'),
                                                                         (4096, 5000, [])]) as unwrap:
            self.assertEqual(lossy_assets.unwrap([obj], args, None), (4096, 5000, []))
        self.assertEqual(args.pack_shape, 'AABB')
        self.assertEqual(unwrap.call_args_list[1].args[1].pack_shape, 'CONVEX')
        layers.remove.assert_called_once_with(layers.get.return_value)
        images.remove.assert_called_once_with(images.new.return_value)

    def test_source_crop_preserves_texel_coordinates_and_triangle_indices(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / 'source.glb'
            source.write_bytes(glb(UNLIT))
            doc, buffers, _ = lossy_assets.read_glb(source)
            uv = np.array([[.2, .3], [.3, .3], [.2, .4]], dtype=np.float32)
            data = bytearray(buffers[0])
            data[36:60] = uv.tobytes()
            buffers = [bytes(data)]
            crops = lossy_assets.source_texture_crops(doc, buffers, {0: [8192, 8192]})
            self.assertLess(np.prod(crops[0]['size']), 8192**2 / 50)
            output = Path(directory) / 'cropped.glb'
            lossy_assets.write_lossy(doc, buffers, None, b'', output,
                                     reencoded={0: b'encoded'}, source_crops=crops)
            result, binary, _ = lossy_assets.read_glb(output)
            prim = result['meshes'][0]['primitives'][0]
            actual = lossy_assets.accessor_array(result, binary, prim['attributes']['TEXCOORD_0'], dequantize=True)
            np.testing.assert_allclose(actual * crops[0]['size'] + crops[0]['box'][:2], uv * 8192, atol=.001)
            np.testing.assert_array_equal(lossy_assets.accessor_array(result, binary, prim['indices']), [0, 1, 2])

    def test_source_crop_preserves_repeat_edges_and_unions_shared_image_users(self):
        doc = {'meshes': [{'primitives': [{'attributes': {'TEXCOORD_0': 0}},
                                        {'attributes': {'TEXCOORD_0': 1}}]}]}
        arrays = [np.array([[.2, .3], [.3, .4]]), np.array([[.6, .5], [.7, .6]])]
        with patch.object(lossy_assets, 'display_texture', return_value=({}, 0, {})), \
                patch.object(lossy_assets, 'accessor_array', side_effect=lambda d, b, i, **kw: arrays[i]):
            crop = lossy_assets.source_texture_crops(doc, [], {0: [8192, 8192]})[0]
            self.assertLess(crop['box'][0], .2 * 8192)
            self.assertGreater(crop['box'][2], .7 * 8192)
            arrays[0][0, 0] = 0
            crop = lossy_assets.source_texture_crops(doc, [], {0: [8192, 8192]})[0]
            self.assertEqual([crop['box'][0], crop['box'][2]], [0, 8192])
            arrays[0][0, 0] = -1
            self.assertEqual(lossy_assets.source_texture_crops(doc, [], {0: [8192, 8192]}), {})

    def test_unwrap_restores_source_images_on_success_and_failure(self):
        source = object()
        square = object()
        node = SimpleNamespace(type='TEX_IMAGE', image=source)
        material = Mock(node_tree=SimpleNamespace(nodes=[node]))
        obj = SimpleNamespace(material_slots=[SimpleNamespace(material=material)])
        images = Mock()
        images.new.return_value = square
        def check_aspect(*_args):
            self.assertIs(node.image, square)
            return (32, 30, [])
        for failure in [False, True]:
            def operation(*args):
                result = check_aspect(*args)
                if failure:
                    raise lossy_assets.UnsafeAtlasError('test packing failure')
                return result
            with patch.dict(sys.modules, {'bpy': SimpleNamespace(data=SimpleNamespace(images=images))}), \
                    patch.object(lossy_assets, 'unwrap_square', side_effect=operation):
                if failure:
                    with self.assertRaises(lossy_assets.UnsafeAtlasError):
                        lossy_assets.unwrap([obj], None, None)
                else:
                    self.assertEqual(lossy_assets.unwrap([obj], None, None), (32, 30, []))
            self.assertIs(node.image, source)
            images.remove.assert_called_with(square)

    def test_rebaking_small_source_textures_cannot_inflate_atlas_memory(self):
        self.assertTrue(lossy_assets.atlas_expansion_exceeded(4096, [(512, 484), (128, 56)], 4))
        # Projection sources may cover an entire map: they still benefit from rebaking.
        self.assertFalse(lossy_assets.atlas_expansion_exceeded(688, [(8192, 8192)], 4))
        # Compare the complete source set, not just its smallest image.
        self.assertFalse(lossy_assets.atlas_expansion_exceeded(4096, [(2048, 2048)] * 2, 4))
        self.assertFalse(lossy_assets.atlas_expansion_exceeded(4096, [(2048, 2048)], 4))
        for maximum in [0, float('nan'), float('inf')]:
            with self.assertRaises(ValueError):
                lossy_assets.atlas_expansion_exceeded(4096, [(512, 512)], maximum)

    def test_collapsed_charts_are_rescued_without_changing_valid_charts(self):
        source = np.array([[[0., 0.], [1., 0.], [0., 1.]]] * 4)
        source[3] = 0
        packed = source.copy()
        packed[1:3] = .5
        rescued, count = lossy_assets.rescue_collapsed_charts(source, packed)
        rescued = rescued.reshape(-1, 3, 2)
        self.assertEqual(count, 2)
        np.testing.assert_array_equal(rescued[0], packed[0])
        np.testing.assert_array_equal(rescued[3], packed[3])
        for face in rescued[1:3]:
            self.assertGreater(abs(np.linalg.det(face[1:] - face[0])), 0)
        self.assertLess(rescued[1, :, 0].max(), rescued[2, :, 0].min())

    def test_outside_tile_layout_is_fitted_with_one_uniform_transform(self):
        class UV:
            def __init__(self, values): self.values = np.array(values, dtype=np.float32).ravel()
            def foreach_get(self, _name, output): output[:] = self.values
            def foreach_set(self, _name, values): self.values[:] = values
        layers = [UV([[.4, 1.2], [1.4, 1.2], [.4, 1.4]]),
                  UV([[1.5, 1.5], [1.8, 1.5], [1.5, 1.8]])]
        objects = [SimpleNamespace(data=SimpleNamespace(loops=[0]*3,
                   uv_layers={lossy_assets.NEW_UV: SimpleNamespace(uv=uv)})) for uv in layers]
        before = np.concatenate([uv.values.copy() for uv in layers]).reshape(-1, 2)
        scale = lossy_assets.fit_atlas_tile(objects, .01)
        after = np.concatenate([uv.values for uv in layers]).reshape(-1, 2)
        self.assertGreaterEqual(after.min(), .009999)
        self.assertLessEqual(after.max(), .990001)
        np.testing.assert_allclose(after - after[0], (before - before[0]) * scale, atol=1e-7)
        lossy_assets.check_atlas_uvs(before, after)

    def test_atlas_writer_rejects_skipped_textured_mesh_even_with_shared_material(self):
        doc, binary, _ = lossy_assets.read_glb(self.root / 'derby/house/model.glb')
        with self.assertRaisesRegex(ValueError, 'Textured primitive 0/0 was not rebuilt'):
            lossy_assets.write_lossy(doc, binary, [], b'avif', self.root / 'bad.glb')

    def test_packed_uvs_cannot_destroy_source_textured_triangles(self):
        source = np.array([[[0., 0.], [1., 0.], [0., 1.]]])
        lossy_assets.check_atlas_uvs(source, source * .5 + .1)
        for damaged in [np.full_like(source, .5), source + 1, source * np.nan]:
            with self.assertRaises(lossy_assets.UnsafeAtlasError):
                lossy_assets.check_atlas_uvs(source, damaged)
        # Degenerate source UVs do not become a new optimizer failure.
        lossy_assets.check_atlas_uvs(np.zeros_like(source), np.zeros_like(source))

    def test_ownership_alpha_is_not_physical_transparency(self):
        for mode, expected in [('OPAQUE', 'NONE'), ('MASK', 'STRAIGHT'), ('BLEND', 'STRAIGHT')]:
            with self.subTest(mode=mode):
                material = {**UNLIT, 'alphaMode': mode}
                path = self.root / 'derby/house/model.glb'
                path.write_bytes(glb(material))
                doc, binary, _ = lossy_assets.read_glb(path)
                loaded = SimpleNamespace(alpha_mode=None)
                bpy = SimpleNamespace(data=SimpleNamespace(images=SimpleNamespace(load=lambda _: loaded)))
                with patch.dict(sys.modules, {'bpy': bpy}):
                    images = lossy_assets.load_images(doc, binary, self.root / 'decoded')
                self.assertIs(images[0], loaded)
                self.assertEqual(loaded.alpha_mode, expected)

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(dir=Path(__file__).resolve().parents[1] / 'work')
        self.root = Path(self.temporary.name) / '3d-assets'
        (self.root / 'derby/house').mkdir(parents=True)
        (self.root / 'derby/house/model.glb').write_bytes(glb(UNLIT))
        self.index = {'version': 1, 'assets': [{'id': 'house', 'model': 'derby/house/model.glb',
                                                 'descriptor': 'derby/house/asset.json', 'label': 'kept'}]}
        (self.root / 'derby/house/asset.json').write_text(json.dumps({
            'id': 'house', 'name': 'House', 'source_map': 'Derby', 'model': 'model.glb', 'tags': ['kept']}))
        (self.root / 'index.json').write_text(json.dumps(self.index))
        self.args = lossy_assets.default_settings()

    def tearDown(self):
        self.temporary.cleanup()

    def fake_derive(self, asset_id, model_path, lossy_path, args, work):
        """Stands in for the Blender derivation: writes a lossy model and its receipt."""
        lossy_path.parent.mkdir(parents=True, exist_ok=True)
        lossy_path.write_bytes(b'lossy:' + sha(model_path.read_bytes()).encode())
        receipt = {'source': sha(model_path.read_bytes()), 'output': sha(lossy_path.read_bytes()),
                   'settings': lossy_assets.settings(args)}
        Path(str(lossy_path) + '.receipt.json').write_text(json.dumps(receipt))
        self.derived.append(asset_id)
        return {'asset_id': asset_id}

    def fake_preview(self, source_path, source_relative, output):
        """Stands in for pipeline/src/preview-model.ts, writing the same receipt chain."""
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_bytes(b'preview:' + Path(source_path).read_bytes()[:8])
        Path(str(output) + '.receipt.json').write_text(json.dumps(
            {'source': sha(Path(source_path).read_bytes()), 'source_model': source_relative,
             'fingerprint': 'fp', 'output': sha(output.read_bytes())}))
        self.previews.append(source_relative)
        return {'bytes': 10, 'edge': 32}

    def refresh(self, previews=False, **options):
        self.derived, self.previews = [], []
        with patch.object(lossy_assets, 'derive', self.fake_derive), \
                patch.object(lossy_assets, 'write_preview', self.fake_preview), \
                patch.object(lossy_assets, 'preview_fingerprint', lambda: 'fp'), \
                patch.object(lossy_assets, 'summary_row', lambda report: report):
            return lossy_assets.refresh_derivatives(self.root, Path(self.temporary.name) / 'work', previews=previews,
                                                    log=lambda message: None, **options)

    def test_pbr_reencoding_preserves_materials_uvs_and_geometry(self):
        material = {'normalTexture': {'index': 0, 'scale': 0.8},
                    'occlusionTexture': {'index': 0, 'strength': 0.7},
                    'pbrMetallicRoughness': {'baseColorTexture': {'index': 0},
                                           'metallicRoughnessTexture': {'index': 0}}}
        source = self.root / 'pbr.glb'
        source.write_bytes(glb(material))
        self.assertEqual(lossy_assets.static_check(self.root, 'pbr.glb'), [])
        doc, binary, _ = lossy_assets.read_glb(source)
        output = self.root / 'pbr.lossy.glb'
        lossy_assets.write_lossy(doc, binary, [], b'', output, drop_normals=False,
                                reencoded={0: b'encoded-image'})
        result, body, _ = lossy_assets.read_glb(output)
        self.assertEqual(result['materials'], doc['materials'])
        original = doc['meshes'][0]['primitives'][0]
        written = result['meshes'][0]['primitives'][0]
        for name, index in original['attributes'].items():
            np.testing.assert_array_equal(lossy_assets.accessor_array(doc, binary, index),
                lossy_assets.accessor_array(result, body, written['attributes'][name]))
        self.assertEqual(result['textures'][0]['extensions']['EXT_texture_avif']['source'], 0)
        self.assertEqual(result['images'][0]['mimeType'], 'image/avif')

    def test_static_check_accepts_display_textures_and_refuses_others(self):
        model = 'derby/house/model.glb'
        self.assertEqual(lossy_assets.static_check(self.root, model), [])
        foliage = dict(UNLIT, emissiveTexture={'index': 0}, emissiveFactor=[1, 1, 1], alphaMode='MASK')
        background = {'pbrMetallicRoughness': {'baseColorFactor': [0, 0, 0, 1]}, 'emissiveTexture': {'index': 0},
                      'emissiveFactor': [1, 1, 1]}
        for material in (foliage, background):
            (self.root / model).write_bytes(glb(material))
            self.assertEqual(lossy_assets.static_check(self.root, model), [])
        for material in ({'pbrMetallicRoughness': {'baseColorFactor': [0.5, 0, 0, 1]}, 'emissiveTexture': {'index': 0},
                          'emissiveFactor': [1, 1, 1]},):
            (self.root / model).write_bytes(glb(material))
            self.assertTrue(lossy_assets.static_check(self.root, model))

    def test_refresh_derives_once_sets_field_and_rederives_changed_models(self):
        report = self.refresh()
        index = json.loads((self.root / 'index.json').read_text())
        self.assertEqual(index['assets'][0]['lossy_model'], 'derby/house/lossy.glb')
        self.assertEqual(index['assets'][0]['tags'], ['kept'])
        self.assertEqual((len(report['derived']), self.derived), (1, ['house']))
        self.assertEqual(lossy_assets.verify_derivatives(self.root), [])
        self.assertEqual(self.refresh()['current'], ['house'])
        self.assertEqual(self.derived, [])
        # Republished model bytes invalidate the receipt until the next refresh.
        (self.root / 'derby/house/model.glb').write_bytes(glb(dict(UNLIT, doubleSided=True)))
        self.assertIn('house: lossy receipt does not bind the current model', lossy_assets.verify_derivatives(self.root))
        self.refresh()
        self.assertEqual(self.derived, ['house'])
        self.assertEqual(lossy_assets.verify_derivatives(self.root), [])

    def test_scoped_refresh_does_not_publish_while_an_unselected_lossy_asset_is_stale(self):
        self.refresh()
        index_path = self.root/'index.json'
        index = json.loads(index_path.read_text())
        (self.root/'other').mkdir()
        (self.root/'other/model.glb').write_bytes(b'original')
        (self.root/'other/lossy.glb').write_bytes(b'stale')
        (self.root/'other/asset.json').write_text(json.dumps({
            'id': 'other', 'name': 'Other', 'source_map': 'Derby', 'model': 'model.glb'}))
        index_path.write_text(json.dumps(index))
        before = index_path.read_bytes()
        with self.assertRaisesRegex((ValueError, RuntimeError), 'other: lossy model or receipt missing'):
            self.refresh(ids={'house'})
        self.assertEqual(index_path.read_bytes(), before)

    def test_disabled_refresh_removes_the_field_and_refusals_keep_none(self):
        self.refresh()
        report = self.refresh(lossy=False)
        self.assertEqual(report['removed'], ['house'])
        self.assertFalse((self.root/'derby/house/lossy.glb').exists())
        self.assertFalse((self.root/'derby/house/lossy.glb.receipt.json').exists())
        self.assertNotIn('lossy_model', json.loads((self.root / 'index.json').read_text())['assets'][0])
        (self.root / 'derby/house/model.glb').write_bytes(glb(dict(UNLIT, emissiveTexture={'index': 0}, emissiveFactor=[0.5, 0.5, 0.5])))
        report = self.refresh()
        self.assertIn('house', report['refused'])
        self.assertNotIn('lossy_model', json.loads((self.root / 'index.json').read_text())['assets'][0])

    def test_changed_settings_or_tampered_output_are_not_current(self):
        self.refresh()
        lossy = 'derby/house/lossy.glb'
        self.assertTrue(lossy_assets.receipt_current(self.root, 'derby/house/model.glb', lossy, self.args))
        self.assertFalse(lossy_assets.receipt_current(self.root, 'derby/house/model.glb', lossy,
                                                      lossy_assets.default_settings(quality=60)))
        # Receipts written before --nearest-density existed lack that setting and re-derive.
        receipt_path = self.root / (lossy + '.receipt.json')
        receipt = json.loads(receipt_path.read_text())
        receipt['settings'].pop('nearest_density')
        receipt_path.write_text(json.dumps(receipt))
        self.assertFalse(lossy_assets.receipt_current(self.root, 'derby/house/model.glb', lossy, self.args))
        (self.root / lossy).write_bytes(b'tampered')
        self.assertFalse(lossy_assets.receipt_current(self.root, 'derby/house/model.glb', lossy, self.args))
        self.assertIn('house: lossy model bytes differ from its receipt', lossy_assets.verify_derivatives(self.root))

    def test_algorithm_revision_invalidates_previous_receipts(self):
        self.refresh()
        path = self.root / 'derby/house/lossy.glb.receipt.json'
        receipt = json.loads(path.read_text())
        receipt['settings'].pop('algorithm_version')
        path.write_text(json.dumps(receipt))
        self.assertFalse(lossy_assets.receipt_current(
            self.root, 'derby/house/model.glb', 'derby/house/lossy.glb', self.args))

    def test_size_preserves_detail_across_surface_instead_of_only_median(self):
        # Half the asset was previously allowed to lose up to 4x its target density.
        size, required = lossy_assets.choose_size(
            self.args, np.ones(3), np.array([.01, .005, .0025]), np.array([51, 40, 9]))
        self.assertEqual((size, required), (400, 400))

    def test_quantization_keeps_thin_geometry_and_uv_triangles_as_floats(self):
        doc, buffers, _ = lossy_assets.read_glb(self.root / 'derby/house/model.glb')
        # A long triangle only 1e-6 wide collapses on the asset's uint16 grid.
        positions = np.array([[0, 0, 0], [1, 0, 0], [1, 1e-6, 0]], dtype=np.float32)
        body = bytearray(buffers[0])
        body[:36] = positions.tobytes()
        buffers = [bytes(body)]
        quantizer = lossy_assets.Quantizer(doc, buffers, 8)
        self.assertFalse(quantizer.quantize_positions)
        output = self.root / 'thin.glb'
        lossy_assets.write_lossy(doc, buffers, None, b'', output,
                                 normal_bits=8, reencoded={0: b'fake avif'})
        written, binary, _ = lossy_assets.read_glb(output)
        index = written['meshes'][0]['primitives'][0]['attributes']['POSITION']
        self.assertEqual(written['accessors'][index]['componentType'], 5126)
        self.assertNotIn('scale', written['nodes'][0])
        np.testing.assert_array_equal(lossy_assets.accessor_array(written, binary, index), positions)
        uv = positions[:, :2]
        _, template = quantizer.convert('TEXCOORD_0', uv, {'componentType': 5126}, [0, 1, 2])
        self.assertEqual(template['componentType'], 5126)
        uv = np.array([[0, 0], [1, 0], [0, 1]], dtype=np.float32)
        _, template = quantizer.convert('TEXCOORD_0', uv, {'componentType': 5126}, [0, 1, 2])
        self.assertEqual(template['componentType'], 5123)

    def test_untextured_derivative_preserves_transformed_geometry_without_quantization(self):
        doc, buffers, _ = lossy_assets.read_glb(self.root / 'derby/house/model.glb')
        doc['images'] = []
        doc['textures'] = []
        doc['materials'] = [{'pbrMetallicRoughness': {'baseColorFactor': [.5, .4, .3, 1]}}]
        doc['nodes'][0]['translation'] = [7, 8, 9]
        original = lossy_assets.accessor_array(doc, buffers, 0)
        output = self.root / 'untextured.glb'
        lossy_assets.write_lossy(doc, buffers, [], b'', output, reencoded={})
        written, binary, _ = lossy_assets.read_glb(output)
        self.assertEqual(written['nodes'], doc['nodes'])
        self.assertEqual(written['materials'], doc['materials'])
        self.assertEqual(written['images'], [])
        self.assertNotIn('EXT_texture_avif', written.get('extensionsRequired', []))
        index = written['meshes'][0]['primitives'][0]['attributes']['POSITION']
        np.testing.assert_array_equal(lossy_assets.accessor_array(written, binary, index), original)
        self.assertEqual(lossy_assets.static_check(self.root, 'untextured.glb', quantize=False), [])
        self.assertTrue(lossy_assets.static_check(self.root, 'untextured.glb'))

    def test_reencoded_duplicate_images_share_one_buffer(self):
        doc, buffers, _ = lossy_assets.read_glb(self.root / 'derby/house/model.glb')
        doc['images'].append(dict(doc['images'][0]))
        output = self.root / 'shared-images.glb'
        lossy_assets.write_lossy(doc, buffers, [], b'', output,
                                 reencoded={0: b'same avif', 1: b'same avif'})
        written, _, _ = lossy_assets.read_glb(output)
        self.assertEqual(written['images'][0]['bufferView'], written['images'][1]['bufferView'])

    def test_triangle_precision_rejects_flips_and_preserves_safe_rounding(self):
        triangle = np.array([[0., 0.], [1., 0.], [0., 1.]])
        self.assertFalse(lossy_assets.triangle_precision_safe(triangle, triangle[[0, 2, 1]], [0, 1, 2]))
        self.assertTrue(lossy_assets.triangle_precision_safe(triangle, triangle + 1e-6, [0, 1, 2]))

    def test_safe_geometry_still_quantizes_and_roundtrips(self):
        doc, buffers, _ = lossy_assets.read_glb(self.root / 'derby/house/model.glb')
        output = self.root / 'safe.glb'
        lossy_assets.write_lossy(doc, buffers, None, b'', output,
                                 normal_bits=8, reencoded={0: b'fake avif'})
        written, binary, _ = lossy_assets.read_glb(output)
        index = written['meshes'][0]['primitives'][0]['attributes']['POSITION']
        self.assertEqual(written['accessors'][index]['componentType'], 5123)
        positions = lossy_assets.accessor_array(written, binary, index, dequantize=True)
        node = written['nodes'][0]
        np.testing.assert_allclose(positions * node['scale'] + node['translation'],
                                   lossy_assets.accessor_array(doc, buffers, 0), atol=1 / 65535)

    def test_previews_follow_the_lossy_model_and_rebuild_when_it_changes(self):
        self.refresh(previews=True)
        entry = json.loads((self.root / 'index.json').read_text())['assets'][0]
        self.assertEqual((entry['preview_model'], self.previews), ('derby/house/preview.glb', ['derby/house/lossy.glb']))
        self.assertEqual(lossy_assets.verify_derivatives(self.root), [])
        self.refresh(previews=True)
        self.assertEqual(self.previews, [])
        (self.root / 'derby/house/model.glb').write_bytes(glb(dict(UNLIT, doubleSided=True)))
        self.refresh(previews=True)
        self.assertEqual((self.derived, self.previews), (['house'], ['derby/house/lossy.glb']))
        # Without a lossy model the preview comes from the model itself.
        self.refresh(previews=True, lossy=False)
        self.assertEqual(self.previews, ['derby/house/model.glb'])
        self.assertEqual(lossy_assets.verify_derivatives(self.root), [])

    def test_unreferenced_textured_materials_are_kept_untextured(self):
        from types import SimpleNamespace
        model = self.root / 'derby/house/model.glb'
        model.write_bytes(glb(UNLIT, extra_materials=[dict(UNLIT, name='spline leftover')]))
        self.assertEqual(lossy_assets.static_check(self.root, 'derby/house/model.glb'), [])
        doc, binary, _ = lossy_assets.read_glb(model)

        class FakeUV:  # The new atlas UVs Blender would hold for the one triangle.
            def __len__(self): return 3
            def foreach_get(self, _name, values): values[:] = [0, 0, 1, 0, 0, 1]
        obj = SimpleNamespace(data=SimpleNamespace(uv_layers={lossy_assets.NEW_UV: SimpleNamespace(uv=FakeUV())}))
        record = {'object': obj, 'mesh': 0, 'corners': [[(0, 0), (0, 1), (0, 2)]],
                  'materials': [(None, 'Linear', 'REPEAT', 0)]}
        output = self.root / 'derby/house/lossy.glb'
        lossy_assets.write_lossy(doc, binary, [record], b'avif', output)
        written, _, _ = lossy_assets.read_glb(output)
        self.assertEqual(written['materials'][0]['pbrMetallicRoughness']['baseColorTexture'], {'index': 0})
        self.assertNotIn('baseColorTexture', written['materials'][1]['pbrMetallicRoughness'])
        self.assertEqual(written['materials'][1]['name'], 'spline leftover')
        self.assertEqual(len(written['textures']), 1)

    def test_preview_receipts_must_bind_the_lossy_model(self):
        self.refresh()
        preview = self.root / 'derby/house/preview.glb'
        preview.write_bytes(b'preview')
        lossy_sha = sha((self.root / 'derby/house/lossy.glb').read_bytes())
        Path(str(preview) + '.receipt.json').write_text(json.dumps(
            {'source': lossy_sha, 'source_model': 'derby/house/lossy.glb', 'output': sha(b'preview')}))
        index = json.loads((self.root / 'index.json').read_text())
        index['assets'][0]['preview_model'] = 'derby/house/preview.glb'
        (self.root / 'index.json').write_text(json.dumps(index))
        self.assertEqual(lossy_assets.verify_derivatives(self.root), [])
        Path(str(preview) + '.receipt.json').write_text(json.dumps(
            {'source': 'f' * 64, 'source_model': 'derby/house/lossy.glb', 'output': sha(b'preview')}))
        self.assertTrue(lossy_assets.verify_derivatives(self.root))

    def test_library_publish_and_rollback_restore_the_previous_state(self):
        run = Path(self.temporary.name) / 'run'
        stage = run / 'stage'
        self.derived, self.previews = [], []
        self.fake_derive('house', self.root / 'derby/house/model.glb', stage / 'derby/house/lossy.glb', self.args, None)
        self.fake_preview(stage / 'derby/house/lossy.glb', 'derby/house/lossy.glb', stage / 'derby/house/preview.glb')
        files = {name: stage / name for name in ('derby/house/lossy.glb', 'derby/house/lossy.glb.receipt.json',
                                                 'derby/house/preview.glb', 'derby/house/preview.glb.receipt.json')}
        fields = {'lossy_model': 'derby/house/lossy.glb', 'preview_model': 'derby/house/preview.glb'}
        (run / 'backup').mkdir(parents=True)
        (run / 'backup/index.json').write_bytes((self.root / 'index.json').read_bytes())
        record = {'root': str(self.root), 'files': [], 'index': [], 'reports': {}, 'failures': {}}
        model_sha = sha((self.root / 'derby/house/model.glb').read_bytes())
        lossy_assets.publish_one(self.root, run, 'house', 'derby/house/model.glb', files, fields, model_sha, record)
        entry = json.loads((self.root / 'index.json').read_text())['assets'][0]
        self.assertEqual((entry['lossy_model'], entry['preview_model']), (fields['lossy_model'], fields['preview_model']))
        self.assertEqual(lossy_assets.verify_derivatives(self.root), [])
        with self.assertRaisesRegex(ValueError, 'Model changed'):
            lossy_assets.publish_one(self.root, run, 'house', 'derby/house/model.glb', files, fields, 'a' * 64,
                                     dict(record, files=[], index=[]))
        lossy_assets.main_rollback(type('Args', (), {'run': run})())
        entry = json.loads((self.root / 'index.json').read_text())['assets'][0]
        self.assertNotIn('lossy_model', entry)
        self.assertNotIn('preview_model', entry)
        self.assertFalse((self.root / 'derby/house/lossy.glb').exists())
        self.assertFalse((self.root / 'derby/house/preview.glb').exists())


if __name__ == '__main__':
    unittest.main()
