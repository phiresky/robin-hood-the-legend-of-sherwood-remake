import unittest
import numpy as np
from restart14_butterfly07_pose_fit import geometry,WING,BODY
from scipy.spatial import cKDTree

class PoseTests(unittest.TestCase):
    def test_depth_mirror_has_identical_source_projection(self):
        p=np.array([27.,-34.,61.,43.,-19.,.5,-.2]);mirror=p.copy();mirror[[0,1,3,4]]*=-1
        a,aw=geometry(p);b,bw=geometry(mirror)
        reflected=a.copy();reflected[:,2]*=-1
        self.assertLess(cKDTree(b).query(reflected)[0].max(),1e-12)
        for x,y in zip(aw,bw):
            np.testing.assert_allclose(x[:,:2],y[:,:2],atol=1e-12)
            np.testing.assert_allclose(x[:,2],-y[:,2],atol=1e-12)

    def test_all_poses_conserve_body_and_wing_dimensions(self):
        for p in [np.zeros(7),np.array([27.,-34.,61.,43.,-19.,.5,-.2])]:
            body,wings=geometry(p)
            for actual,rest in [(body,BODY),*[(w,WING) for w in wings]]:
                np.testing.assert_allclose(np.linalg.norm(actual[:,None]-actual[None,:],axis=2),np.linalg.norm(rest[:,None]-rest[None,:],axis=2),atol=1e-12)

if __name__=='__main__':unittest.main()
