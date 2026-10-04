import tempfile
import unittest
from pathlib import Path
from PIL import Image
from render_convergence import difference


class RenderConvergenceTest(unittest.TestCase):
    def test_opaque_black_ray_failure_is_not_hidden_by_zero_rgb(self):
        with tempfile.TemporaryDirectory() as folder:
            a, b = Path(folder) / 'a.png', Path(folder) / 'b.png'
            Image.new('RGBA', (1, 1), (0, 0, 0, 255)).save(a)
            Image.new('RGBA', (1, 1), (0, 0, 0, 0)).save(b)
            result = difference(a, b)
            self.assertEqual(result['alpha_threshold_changed_pixels'], 1)
            self.assertEqual(result['opaque_rgb_mean_delta'], 0)
            self.assertFalse(result['rgba_exact'])

    def test_shading_drift_is_separate_from_silhouette(self):
        with tempfile.TemporaryDirectory() as folder:
            a, b = Path(folder) / 'a.png', Path(folder) / 'b.png'
            Image.new('RGBA', (1, 1), (10, 20, 30, 255)).save(a)
            Image.new('RGBA', (1, 1), (13, 23, 33, 255)).save(b)
            result = difference(a, b)
            self.assertEqual(result['alpha_threshold_changed_pixels'], 0)
            self.assertEqual(result['opaque_rgb_mean_delta'], 3)
            self.assertFalse(result['rgba_exact'])

    def test_rejects_different_framing_dimensions(self):
        with tempfile.TemporaryDirectory() as folder:
            a, b = Path(folder) / 'a.png', Path(folder) / 'b.png'
            Image.new('RGBA', (1, 1)).save(a)
            Image.new('RGBA', (2, 1)).save(b)
            with self.assertRaises(ValueError):
                difference(a, b)


if __name__ == '__main__':
    unittest.main()
