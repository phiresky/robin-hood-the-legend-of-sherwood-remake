import unittest
import numpy as np
from shapely import union_all
from shapely.geometry import Polygon, box
from native_occlusion_proof import ObservedOcclusion, opaque_footprint


class OcclusionProofTests(unittest.TestCase):
    def test_closer_complete_cover_is_required(self):
        target = box(1, 1, 2, 2)
        self.assertTrue(ObservedOcclusion([box(0, 0, 3, 3)], [10]).prove(target, 5)['hidden'])
        self.assertFalse(ObservedOcclusion([box(0, 0, 3, 3)], [4]).prove(target, 5)['hidden'])

    def test_native_hole_cannot_be_declared_hidden(self):
        target = box(1, 1, 2, 2)
        occluder = box(0, 0, 3, 3).difference(box(1.4, 1.4, 1.6, 1.6))
        self.assertFalse(ObservedOcclusion([occluder], [10]).prove(target, 5)['hidden'])

    def test_boundary_uncertainty_retains_protection(self):
        target = box(1, 1, 2, 2)
        self.assertFalse(ObservedOcclusion([target], [10]).prove(target, 5)['hidden'])

    def test_joint_overlapping_observed_fragments_can_cover(self):
        target = box(1, 1, 2, 2)
        self.assertTrue(ObservedOcclusion([box(0, 0, 1.7, 3), box(1.3, 0, 3, 3)], [10, 10]).prove(target, 5)['hidden'])

    def test_alpha_hole_and_affine_uv_projection(self):
        alpha = np.array([[1, 0], [1, 1]], bool)
        uv = np.array([[0, 0], [1, 0], [0, 1]])
        projected = np.array([[10, 20], [14, 20], [10, 24]])
        footprint = opaque_footprint(uv, alpha, projected)
        self.assertTrue(footprint.covers(box(10.2, 20.2, 10.4, 20.4)))
        self.assertFalse(footprint.intersects(box(12.2, 20.2, 12.4, 20.4)))
        self.assertTrue(Polygon(projected).covers(footprint))

    def test_target_clipping_matches_full_union_with_fragmented_alpha(self):
        rng = np.random.default_rng(17)
        for _ in range(20):
            patches = [box(x, y, x + .7, y + .9) for x, y in rng.uniform(-2, 2, (30, 2))]
            authority = ObservedOcclusion(patches, [10] * len(patches))
            target = box(-.3, -.4, .5, .6)
            full_remainder = target.buffer(authority.margin).difference(union_all(authority.polygons))
            proof = authority.prove(target, 5)
            self.assertEqual(proof['hidden'], full_remainder.is_empty)
            self.assertAlmostEqual(proof['uncovered_area'], full_remainder.area, places=12)


if __name__ == '__main__':
    unittest.main()
