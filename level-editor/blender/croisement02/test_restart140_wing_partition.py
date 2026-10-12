"""Check that fore/hind partition preserves projected rest silhouette and coverage."""
import unittest
import numpy as np
from scipy.spatial.transform import Rotation
from matplotlib.path import Path as Polygon
from restart106_butterfly_four_wing_probe import split_y

class WingPartition(unittest.TestCase):
 def test_projected_union_preserved(self):
  wing=np.array([[0,-1,0],[1,-5,0],[4,-6,0],[7,-2,0],[6,3,0],[3,5,0],[0,1,0]],float)
  halves=[split_y(wing,side) for side in [True,False]]
  q=np.array([[x,y] for x in np.arange(-10,10,.17)+.031 for y in np.arange(-10,10,.19)+.073])
  rng=np.random.default_rng(140)
  for rotation in Rotation.random(25,random_state=rng):
   whole=Polygon(rotation.apply(wing)[:,:2]).contains_points(q)
   divided=np.logical_or.reduce([Polygon(rotation.apply(h)[:,:2]).contains_points(q) for h in halves])
   np.testing.assert_array_equal(whole,divided)
 def test_boundary_remains_at_root_plane(self):
  wing=np.array([[0,-1,0],[4,-5,0],[7,1,0],[3,5,0],[0,1,0]],float)
  fore,hind=split_y(wing,True),split_y(wing,False)
  self.assertTrue(np.all(fore[:,1]<=0));self.assertTrue(np.all(hind[:,1]>=0))
  np.testing.assert_array_equal(fore[fore[:,1]==0],hind[hind[:,1]==0])

if __name__=='__main__':unittest.main()
