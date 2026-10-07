import unittest
import numpy as np
from scipy.interpolate import CubicSpline
from restart26_butterfly07_depth_curve import derivative_bounds

class DepthCurveTests(unittest.TestCase):
    def test_analytic_bounds_cover_dense_periodic_curve(self):
        times=np.arange(9);heights=np.array([0,1,2,.3,-1,.2,1,-.2,0])
        curve=CubicSpline(times,heights,bc_type='periodic')
        speed,acceleration=derivative_bounds(curve)
        dense=np.linspace(0,8,20001)
        self.assertGreaterEqual(speed,np.abs(curve(dense,1)).max()-1e-10)
        self.assertGreaterEqual(acceleration,np.abs(curve(dense,2)).max()-1e-10)
        self.assertLess(speed-np.abs(curve(dense,1)).max(),1e-5)
        self.assertLess(acceleration-np.abs(curve(dense,2)).max(),1e-5)

if __name__=='__main__':unittest.main()
