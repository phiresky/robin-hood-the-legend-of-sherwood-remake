import unittest
import numpy as np
from fill_physical_foliage import triangle_pixels, fill_atlas_edges


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

    def test_subpixel_grid_covers_thin_triangle_without_leaving_surface(self):
        uv = np.array([(0., 0.), (.24, 0.), (0., 1.)])
        self.assertEqual(len(triangle_pixels(uv, 1, 1)[0]), 0)
        samples = 0
        for y in range(4):
            for x in range(4):
                offset = ((x + .5) / 4 - .5, (y + .5) / 4 - .5)
                rows, cols, weights = triangle_pixels(uv, 1, 1, offset)
                self.assertTrue((weights >= -1e-7).all())
                np.testing.assert_allclose(weights @ uv,
                    np.column_stack([cols + .5, rows + .5]) + offset)
                samples += len(rows)
        self.assertGreater(samples, 0)

    def test_edge_fill_is_bounded_and_never_propagates_or_changes_alpha(self):
        colors = np.zeros((1, 20, 4), dtype=np.float32)
        colors[..., 3] = .7
        colors[0, 8, :3] = [.1, .7, .2]
        generated = np.zeros((1, 20), bool); generated[0, 8] = True
        eligible = np.ones((1, 20), bool); eligible[0, 7] = False
        result, repaired = fill_atlas_edges(colors, generated, eligible, 2)
        self.assertEqual(np.flatnonzero(repaired).tolist(), [6, 9, 10])
        np.testing.assert_array_equal(result[..., 3], colors[..., 3])
        np.testing.assert_array_equal(result[~repaired], colors[~repaired])
        np.testing.assert_array_equal(result[0, 10], colors[0, 8])
        self.assertFalse(repaired[0, 11])

    def test_edge_fill_rejects_large_repairs_and_has_no_source_donors(self):
        colors = np.ones((3, 3, 4), dtype=np.float32)
        eligible = np.ones((3, 3), bool)
        generated = np.zeros((3, 3), bool)
        _, repaired = fill_atlas_edges(colors, generated, eligible, 2)
        self.assertFalse(repaired.any())
        generated[1, 1] = True
        with self.assertRaisesRegex(ValueError, '20%'):
            fill_atlas_edges(colors, generated, eligible, 2)

    def test_degenerate_uv_fails(self):
        with self.assertRaisesRegex(ValueError, 'Degenerate'):
            triangle_pixels([(0, 0), (.5, .5), (1, 1)], 8, 8)


if __name__ == '__main__':
    unittest.main()
