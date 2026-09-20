"""Checks for the inferred valley's continuous, monotone depth profile."""
import unittest

from derby_background import _depth_at, height_at


class ValleyProfileTests(unittest.TestCase):
    def test_depth_remains_monotone_and_meets_plateau(self):
        samples = [_depth_at(i/4) for i in range(2721)]
        self.assertEqual(samples[0], -900)
        self.assertEqual(samples[-1], 0)
        self.assertTrue(all(a <= b <= 0 for a, b in zip(samples, samples[1:])))
        self.assertAlmostEqual((_depth_at(680)-_depth_at(679.999))/.001, 0, places=4)

    def test_internal_boundaries_do_not_create_flat_terraces(self):
        for y in (180, 420):
            left = (_depth_at(y)-_depth_at(y-.001))/.001
            right = (_depth_at(y+.001)-_depth_at(y))/.001
            self.assertGreater(left, .5)
            self.assertAlmostEqual(left, right, places=4)

    def test_background_does_not_extend_into_castle_footing(self):
        for x in range(0, 1921, 80):
            self.assertEqual(height_at(x, 680), 0)
            self.assertEqual(height_at(x, 2000), 0)
            for y in range(0, 680, 20):
                self.assertLessEqual(height_at(x, y), 0)


if __name__ == '__main__':
    unittest.main()
