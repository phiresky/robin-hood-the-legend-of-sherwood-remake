import unittest
import numpy as np
from verify_texture_combine import editable_texels


class EditableTexelsTests(unittest.TestCase):
    def test_terrain_ownership_protects_observed_gray_and_color(self):
        data = np.array([[[128, 128, 128, 0], [0, 20, 40, 0],
                          [128, 128, 128, 255], [20, 30, 40, 255],
                          [128, 128, 128, 128]]], dtype=np.uint8)
        np.testing.assert_array_equal(editable_texels(data, terrain=True),
                                      [[True, True, False, False, False]])

    def test_other_atlases_require_opaque_neutral(self):
        data = np.array([[[128, 128, 128, 0], [128, 128, 128, 255],
                          [128, 127, 128, 255], [128, 128, 128, 128]]], dtype=np.uint8)
        np.testing.assert_array_equal(editable_texels(data), [[False, True, False, False]])


if __name__ == '__main__':
    unittest.main()
