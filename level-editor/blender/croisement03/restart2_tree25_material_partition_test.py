import unittest
from restart2_tree25_material_partition import partition


class PartitionTest(unittest.TestCase):
    def test_material_mixed_faces_are_partitioned_without_loss(self):
        known, unknown = partition({8: [1., 1., 1.], 19: [0., 0., 0.], 27: [1.] * 4})
        self.assertEqual(known, [8, 27])
        self.assertEqual(unknown, [19])

    def test_crossing_face_must_not_be_reclassified(self):
        for flags in ([0., 1., 0.], [.5, .5, .5], []):
            with self.subTest(flags=flags), self.assertRaises(ValueError):
                partition({7: flags})

    def test_homogeneous_material_needs_no_partition(self):
        self.assertEqual(partition({3: [1., 1., 1.]}), ([3], []))
        self.assertEqual(partition({3: [0., 0., 0.]}), ([], [3]))


if __name__ == '__main__':
    unittest.main()
