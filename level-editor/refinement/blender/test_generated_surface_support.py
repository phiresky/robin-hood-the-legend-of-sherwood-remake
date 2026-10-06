import unittest
import numpy as np
from generated_surface_support import support, filtered_color, edit_support


class GeneratedSupportTests(unittest.TestCase):
    def test_connected_background_rejected_but_window_preserved(self):
        image = np.zeros((12, 12, 4))
        surface = np.zeros((12, 12), bool)
        surface[2:10, 2:10] = True
        image[2:9, 2:10, :3] = .4
        image[5, 5, :3] = 0
        result = support(image, surface, .01)
        self.assertFalse(result[9, 5])
        self.assertTrue(result[5, 5])
        self.assertTrue(result[8, 5])

    def test_bilinear_background_does_not_darken_valid_edge(self):
        result = filtered_color([[.5, .4, .3], [0, 0, 0]], [.25, .75], [True, False])
        np.testing.assert_allclose(result, [.5, .4, .3])
        self.assertIsNone(filtered_color([[0, 0, 0]], [1], [False]))

    def test_edit_support_excludes_protected_gray_border(self):
        ownership = np.zeros((1, 2, 4))
        edit = ownership.copy(); edit[0, 1, 3] = 1
        valid = edit_support(edit, ownership)
        np.testing.assert_allclose(filtered_color([[.4,.2,.1],[.65,.65,.65]],[.2,.8],valid[0]),[.4,.2,.1])

    def test_edit_support_cannot_claim_known_source(self):
        ownership = np.zeros((1, 2, 4)); ownership[0, 0, 3] = 1
        with self.assertRaises(ValueError): edit_support(np.zeros((1,2,4)), ownership)

    def test_edit_support_rejects_soft_or_empty_masks(self):
        ownership = np.zeros((1, 2, 4)); edit = ownership.copy(); edit[0,0,3] = .5
        with self.assertRaises(ValueError): edit_support(edit, ownership)
        edit[:,:,3] = 1
        with self.assertRaises(ValueError): edit_support(edit, ownership)


if __name__ == '__main__':
    unittest.main()
