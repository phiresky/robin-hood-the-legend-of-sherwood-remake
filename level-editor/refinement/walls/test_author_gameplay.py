import unittest
import numpy as np
from author_gameplay import strip_gameplay


class WallGameplayTests(unittest.TestCase):
    def test_straight_barrier_compacts_and_preserves_physical_policy(self):
        triangles = [np.array([[0., -3, 0], [20, 3, 0], [20, -3, 10]]),
                     np.array([[0., -3, 10], [20, 3, 10], [0, 3, 0]])]
        before = [t.copy() for t in triangles]
        gameplay = strip_gameplay(triangles, 'wall', material=3, opaque=False)
        self.assertTrue(gameplay['volumes'])
        for volume in gameplay['volumes']:
            self.assertTrue(volume['shape']['solid'])
            self.assertFalse(volume['shape']['opaque'])
            self.assertEqual(volume['shape']['default_material'], 3)
            self.assertTrue(all(p['z_bottom'] == 0 for p in volume['shape']['points']))
        for a, b in zip(before, triangles):
            np.testing.assert_array_equal(a, b)

    def test_empty_or_flat_source_does_not_invent_height(self):
        with self.assertRaisesRegex(ValueError, 'nonzero'):
            strip_gameplay([np.array([[0., 0, 0], [10, 0, 0], [10, 3, 0]])], 'wall', material=3, opaque=True)
