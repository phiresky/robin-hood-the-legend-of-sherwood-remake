"""A continuous terrain atlas must not receive another triangle's gutter."""
import unittest
import numpy as np
import global_reproject as gr
from texture_unseen_fill import island_receivers, displayed


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


class RevealVisibilityTests(unittest.TestCase):
    def test_either_roof_trigger_shows_shared_receiver(self):
        receiver = {'reveal_show_when_applied': ['roof-a', 'roof-b'],
                    'reveal_hide_when_applied': ['room-replaced']}
        self.assertFalse(displayed(receiver, set()))
        self.assertTrue(displayed(receiver, {'roof-a'}))
        self.assertTrue(displayed(receiver, {'roof-b'}))
        self.assertTrue(displayed(receiver, {'roof-a', 'roof-b'}))
        self.assertFalse(displayed(receiver, {'roof-a', 'room-replaced'}))

    def test_empty_show_list_fails_like_editor_preview(self):
        with self.assertRaises(ValueError):
            displayed({'reveal_show_when_applied': []}, set())


if __name__ == '__main__':
    unittest.main()
