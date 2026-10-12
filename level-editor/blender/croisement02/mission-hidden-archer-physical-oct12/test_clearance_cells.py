import unittest
import numpy as np
from clearance_cells_v4 import cell_polygon
class CellBoundsTests(unittest.TestCase):
 def test_crossing_triangle_depth_is_bounded(self):
  tri=np.array([[-1,-1,0],[3,-1,8],[-1,3,12]],float)
  clipped=cell_polygon(tri,0,0)
  self.assertEqual(clipped.shape[1],5)
  self.assertGreaterEqual(len(clipped),3)
  self.assertTrue(np.all(clipped[:,:2]>=-1e-7))
  self.assertTrue(np.all(clipped[:,:2]<=1+1e-7))
  self.assertAlmostEqual(float(clipped[:,2].max()),10,places=5)
 def test_disjoint_cell_empty(self):
  self.assertEqual(len(cell_polygon(np.array([[0,0,1],[1,0,1],[0,1,1]],float),5,5)),0)
if __name__=='__main__':unittest.main()
