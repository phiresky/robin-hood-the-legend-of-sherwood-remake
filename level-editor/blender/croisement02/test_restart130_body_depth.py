"""Analytic ownership tests for independently rotated ellipsoid body parts."""
import unittest
import numpy as np
from scipy.spatial.transform import Rotation
from restart130_segmented_body_observations import front_depth

class BodyRayDepth(unittest.TestCase):
 def test_rotated_visible_surface_returns_its_camera_depth(self):
  rng=np.random.default_rng(130);directions=rng.normal(size=(400,3));directions/=np.linalg.norm(directions,axis=1)[:,None];radii=np.array([.42,.55,.42]);center=np.array([0,-3.2,0]);part=dict(radii=radii.tolist(),center=center.tolist())
  for rotation in Rotation.random(12,random_state=rng).as_matrix():
   vertices=(directions*radii+center)@rotation.T;normals=(directions/radii)@rotation.T;visible=normals[:,2]>.01
   actual=front_depth(vertices[visible,:2],rotation,part);np.testing.assert_allclose(actual,vertices[visible,2],atol=1e-10)
 def test_camera_ray_outside_body_has_no_owner(self):
  result=front_depth(np.array([[2.,0],[0,2]]),np.eye(3),dict(center=[0,0,0],radii=[1,1,1]));self.assertTrue(np.all(np.isneginf(result)))
 def test_frontmost_overlapping_body_part_can_be_selected(self):
  xy=np.array([[0.,0.]])
  rear=front_depth(xy,np.eye(3),dict(center=[0,0,-2],radii=[1,1,1]));front=front_depth(xy,np.eye(3),dict(center=[0,0,2],radii=[1,1,1]));self.assertEqual(rear[0],-1);self.assertEqual(front[0],3)

if __name__=='__main__':unittest.main()
