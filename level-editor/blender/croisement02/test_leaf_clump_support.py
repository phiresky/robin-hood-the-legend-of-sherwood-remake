"""Counterexamples for exact support at clipped triangle intersections."""
import math
import unittest
import numpy as np
from leaf_clump_support import triangle,clearance

class SupportTests(unittest.TestCase):
    def setUp(self):
        self.sine=math.sin(math.radians(35));self.cosine=math.cos(math.radians(35));self.ray=np.array([0.,-self.cosine,self.sine])
    def make(self,xy,depth):
        xy=np.array(xy,dtype=float);d=np.array(depth,dtype=float)
        points=np.column_stack((xy[:,0],-self.sine*xy[:,1]-self.cosine*d,-self.cosine*xy[:,1]+self.sine*d))
        return triangle(points,self.ray,self.sine,self.cosine)
    def test_clipped_maximum_is_not_a_body_corner(self):
        body=self.make([[0,0],[4,0],[0,4]],[0,0,0]);receiver=self.make([[1,-1],[1,3],[3,1]],[1,1,3])
        for indices in ([0,1,2],[2,1,0]):
            r={**receiver,'xy':receiver['xy'][indices]};shift,witness,pairs=clearance([body],[r]);self.assertAlmostEqual(shift,3.02);self.assertEqual(pairs,1);np.testing.assert_allclose(witness['screen'],[3,1],atol=1e-10)
    def test_native_ray_shift_preserves_domain_and_closes_clearance(self):
        a=self.make([[0,0],[4,0],[0,4]],[0,0,0]);b=self.make([[0,0],[4,0],[0,4]],[2,2,2]);r=self.make([[0,0],[4,0],[0,4]],[1,1,1]);np.testing.assert_allclose(a['xy'],b['xy']);self.assertAlmostEqual(clearance([a],[r])[0],1.02);self.assertEqual(clearance([b],[r])[0],0.)
    def test_no_receiver_does_not_return_fake_contact(self):
        body=self.make([[0,0],[1,0],[0,1]],[0,0,0]);receiver=self.make([[5,5],[6,5],[5,6]],[0,0,0])
        with self.assertRaisesRegex(ValueError,'No supporting receiver'):clearance([body],[receiver])
    def test_ray_parallel_boundary_is_not_an_affine_surface(self):
        points=np.array([[0.,0.,0.],[1.,0.,0.],self.ray])
        self.assertIsNone(triangle(points,self.ray,self.sine,self.cosine))
if __name__=='__main__':unittest.main()
