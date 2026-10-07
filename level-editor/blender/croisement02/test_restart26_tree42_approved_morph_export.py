import unittest
import numpy as np
from restart26_tree42_approved_morph_export import map_deltas,phase_weights

class MorphExportTests(unittest.TestCase):
    def test_native_holds_and_closing_basis(self):
        times,weights=phase_weights()
        self.assertEqual(weights.shape,(15,13))
        self.assertTrue(np.allclose(times*25,np.arange(15)*4,atol=2e-6))
        self.assertTrue(np.array_equal(weights[0],weights[-1]))
        self.assertTrue(np.array_equal(weights[1:14],np.eye(13)))

    def test_world_delta_maps_through_rotated_scaled_node(self):
        source=np.array([[0.,0.,0.],[1.,2.,3.]])
        delta=np.array([[[.1,.2,.3],[.4,.5,.6]]])
        linear=np.array([[0.,0.,2.],[0.,3.,0.],[-1.,0.,0.]])
        mapped,error=map_deltas(source[::-1],source,delta,linear)
        self.assertEqual(error,0.)
        np.testing.assert_allclose(mapped@linear.T,delta[:,::-1])

    def test_mismatched_basis_fails(self):
        with self.assertRaises(AssertionError):
            map_deltas(np.array([[.2,0.,0.]]),np.zeros((1,3)),np.zeros((13,1,3)),np.eye(3))

    def test_coincident_vertices_with_different_motion_fail(self):
        source=np.zeros((2,3));delta=np.zeros((13,2,3));delta[:,1,0]=.1
        with self.assertRaises(AssertionError):map_deltas(source[:1],source,delta,np.eye(3))

if __name__=='__main__':unittest.main()
