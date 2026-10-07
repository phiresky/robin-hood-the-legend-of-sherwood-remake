"""Check conserved anatomy and camera registration, independent of fitted poses."""
import math, unittest
import numpy as np
from scipy.spatial.distance import pdist
from restart21_butterfly07_geometry_v2 import BODY, WING, geometry

class GeometryTests(unittest.TestCase):
    def test_articulation_conserves_body_and_each_wing(self):
        for p in [[0,0,0,0,0,0,0],[61,-45,97,84,-67,1,-1],[-70,34,-126,-80,80,-2,2]]:
            body,wings=geometry(p)
            np.testing.assert_allclose(pdist(body),pdist(BODY),atol=1e-12)
            for wing in wings:np.testing.assert_allclose(pdist(wing),pdist(WING),atol=1e-12)
    def test_local_screen_registration_does_not_modify_anatomy(self):
        a=geometry([20,30,40,70,-15,0,0]);b=geometry([20,30,40,70,-15,2,-2])
        np.testing.assert_array_equal(a[0],b[0])
        for x,y in zip(a[1],b[1]):np.testing.assert_array_equal(x,y)
    def test_camera_ray_depth_preserves_native_projection(self):
        sine=math.sin(math.radians(35));cosine=math.cos(math.radians(35))
        local=np.vstack(geometry([20,30,40,70,-15,0,0])[1]);screen=local[:,:2]+[1.2,-.4]
        for raydepth in [-12,0,37]:
            d=local[:,2]+raydepth
            world=np.c_[screen[:,0],-sine*screen[:,1]-cosine*d,-cosine*screen[:,1]+sine*d]
            recovered=np.c_[world[:,0],-sine*world[:,1]-cosine*world[:,2]]
            np.testing.assert_allclose(recovered,screen,atol=1e-12)

if __name__=='__main__':unittest.main()
