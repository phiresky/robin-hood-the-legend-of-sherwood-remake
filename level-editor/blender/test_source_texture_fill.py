"""Run with Python + NumPy; does not require Blender."""
import unittest
import numpy as np
from source_texture_fill import donor_patch, fill_island


class SourceTextureFillTests(unittest.TestCase):
    def test_donors_never_include_unknown_or_padding(self):
        rgba = np.full((20, 20, 4), .91, dtype=np.float32)
        owned = np.zeros((20, 20), dtype=bool)
        owned[3:11, 4:12] = True
        rgba[owned, :3] = .17
        patch = donor_patch(rgba, owned)
        self.assertEqual(patch.shape, (8, 8, 3))
        self.assertTrue(np.all(patch == np.float32(.17)))
        self.assertIsNone(donor_patch(rgba, np.zeros_like(owned)))

    def test_fill_preserves_source_and_ownership_exactly(self):
        rgba = np.random.default_rng(5).random((32, 24, 4), dtype=np.float32)
        rgba[:, :, 3] = 0
        rgba[4:20, 3:12, 3] = 1
        before = rgba.copy()
        count = fill_island(rgba, [(np.full((8, 8, 3), .3), .05, 'wall')], .1, 'wall', (4, 9))
        known = before[:, :, 3] == 1
        self.assertEqual(count, int((~known).sum()))
        np.testing.assert_array_equal(rgba[known], before[known])
        np.testing.assert_array_equal(rgba[:, :, 3], before[:, :, 3])
        self.assertTrue(np.all(rgba[~known, :3] == np.float32(.3)))

    def test_similar_surface_wins_over_unrelated_material_in_same_mesh(self):
        rgba = np.zeros((3, 3, 4), dtype=np.float32)
        fill_island(rgba, [(np.ones((8, 8, 3)), 0, 'roof'),
                           (np.full((8, 8, 3), .6), .9, 'other-roof')], .85, 'roof', (0, 0))
        self.assertTrue(np.all(rgba[:, :, :3] == np.float32(.6)))

    def test_missing_donor_keeps_neutral(self):
        rgba = np.full((3, 3, 4), .24, dtype=np.float32)
        before = rgba.copy()
        self.assertEqual(fill_island(rgba, [], 0, 'wall', (0, 0)), 0)
        np.testing.assert_array_equal(rgba, before)


if __name__ == '__main__':
    unittest.main()
