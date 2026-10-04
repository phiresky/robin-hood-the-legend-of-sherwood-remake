"""A continuous terrain atlas must not receive another triangle's gutter."""
import unittest
import numpy as np
import global_reproject as gr
from texture_unseen_fill import island_receivers


class TerrainFillCoverageTests(unittest.TestCase):
    def test_shared_atlas_keeps_opposite_triangle_untouched(self):
        # Two triangles cover a square. The island iterator includes both the
        # first triangle and its surrounding rectangle as potential gutter.
        uv = np.array([[0, 0], [1, 0], [0, 1]], dtype=float)
        record = dict(polygons=np.array([0]), loops=np.array([[0, 1, 2]]),
                      corners=np.array([[[0, 0, 0], [8, 0, 0], [0, 8, 0]]], dtype=float),
                      normals=np.array([[0, 0, 1]], dtype=float))
        face, rows, cols, positions, normals, interior = next(gr.islands(record, uv, (8, 8), lambda group: True))
        selected = island_receivers('ground', np.ones(len(rows), dtype=bool), interior)
        self.assertTrue(np.any(~interior), 'Fixture must include neighbour texels')
        self.assertTrue(selected.any())
        self.assertFalse(np.any(selected & ((rows + cols) > 7)))
        # Independently unwrapped object islands still need their existing gutter.
        self.assertTrue(island_receivers('ownership', np.ones(len(rows), dtype=bool), interior).all())


if __name__ == '__main__':
    unittest.main()
