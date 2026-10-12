import unittest
import numpy as np
from measure_canopy_branch_neighborhoods import triangle_distance
class TriangleDistanceTests(unittest.TestCase):
 def setUp(self):self.triangle=np.array([[[0.,0.,0.],[2.,0.,0.],[0.,2.,0.]]])
 def test_interior_projection(self):self.assertAlmostEqual(triangle_distance(np.array([.5,.5,3.]),self.triangle)[0],3.)
 def test_edge_region(self):self.assertAlmostEqual(triangle_distance(np.array([1.5,1.5,0.]),self.triangle)[0],2**-.5)
 def test_vertex_region(self):self.assertAlmostEqual(triangle_distance(np.array([-1.,-1.,0.]),self.triangle)[0],2**.5)
 def test_degenerate_edge(self):self.assertAlmostEqual(triangle_distance(np.array([1.,1.,0.]),np.array([[[0.,0.,0.],[2.,0.,0.],[2.,0.,0.]]]))[0],1.)
 def test_degenerate_point(self):self.assertAlmostEqual(triangle_distance(np.array([1.,2.,2.]),np.zeros((1,3,3)))[0],3.)
if __name__=='__main__':unittest.main()
