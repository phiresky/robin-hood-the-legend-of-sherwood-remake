import unittest
import numpy as np
from fill_physical_foliage import triangle_pixels


class FoliageAtlasTests(unittest.TestCase):
    def test_two_triangles_cover_atlas_and_reconstruct_centres(self):
        covered = set()
        for uv in ([(0, 0), (1, 0), (1, 1)], [(0, 0), (1, 1), (0, 1)]):
            rows, cols, weights = triangle_pixels(uv, 8, 6)
            np.testing.assert_allclose(weights @ np.asarray(uv) * [8, 6],
                                       np.column_stack([cols + .5, rows + .5]))
            covered.update(zip(rows, cols))
        self.assertEqual(covered, {(y, x) for y in range(6) for x in range(8)})

    def test_clipping_never_wraps_outside_atlas(self):
        rows, cols, _ = triangle_pixels([(-1, -1), (2, -1), (.5, 2)], 5, 7)
        self.assertTrue(((rows >= 0) & (rows < 7)).all())
        self.assertTrue(((cols >= 0) & (cols < 5)).all())

    def test_degenerate_uv_fails(self):
        with self.assertRaisesRegex(ValueError, 'Degenerate'):
            triangle_pixels([(0, 0), (.5, .5), (1, 1)], 8, 8)


if __name__ == '__main__':
    unittest.main()
