import copy
import unittest
from sign_shadow_material import native_shadow_metadata, qualify_native_shadow_export


class NativeShadowMaterialTests(unittest.TestCase):
    def test_legacy_receiver_retains_mask_without_foliage_claim(self):
        node = {'name': 'Native shadow receiver 00', 'mesh': 0,
                'extras': {'source_node': 'mission-panneau-shadow'}}
        material = {'name': 'Native black ground stroke 00', 'doubleSided': True,
                    'alphaMode': 'MASK', 'alphaCutoff': 0.5,
                    'pbrMetallicRoughness': {'baseColorFactor': [1, 1, 1, 1],
                        'baseColorTexture': {'index': 384}, 'metallicFactor': 0, 'roughnessFactor': 1},
                    'extensions': {'KHR_materials_unlit': {}},
                    'extras': {'foliage_physical_opacity': True, 'foliage_alpha_cutoff': 0.5,
                               'opacity_semantics': 'physical-coverage'}}
        original = copy.deepcopy(material)
        qualify_native_shadow_export(node, material)
        self.assertEqual(material['extras'], {'opacity_semantics': 'physical-coverage'})
        self.assertEqual({k:v for k,v in material.items() if k != 'extras'},
                         {k:v for k,v in original.items() if k != 'extras'})
        self.assertNotIn('COLOR_0', node)
        qualify_native_shadow_export(node, material)
        self.assertEqual(material['extras'], {'opacity_semantics': 'physical-coverage'})

    def test_new_authoring_uses_only_physical_coverage(self):
        material = {}
        native_shadow_metadata(material)
        self.assertEqual(material, {'opacity_semantics': 'physical-coverage'})

    def test_foliage_ownership_is_not_silently_discarded(self):
        material = {'foliage_physical_opacity': True,
                    'source_ownership_semantics': 'separate-mask',
                    'source_ownership_channel': 'vertex-color-r'}
        before = dict(material)
        with self.assertRaisesRegex(ValueError, 'separate foliage'):
            native_shadow_metadata(material)
        self.assertEqual(material, before)

    def test_unrelated_node_cannot_be_relabelled(self):
        with self.assertRaisesRegex(ValueError, 'native shadow receiver'):
            qualify_native_shadow_export({'extras': {'source_node': 'foliage-tree'}}, {})


if __name__ == '__main__':
    unittest.main()
