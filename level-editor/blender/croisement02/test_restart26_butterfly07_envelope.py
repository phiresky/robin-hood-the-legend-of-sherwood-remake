import unittest
import numpy as np
from scipy.interpolate import CubicSpline
from restart26_butterfly07_clearance_envelope import forbidden_height,MARGIN_Z
from restart26_butterfly07_envelope_path import free_bands,band_paths,solve_bands
from restart14_butterfly_canopy22_audit import SIN

class ContinuousEnvelopeTests(unittest.TestCase):
    def test_height_band_contains_every_possible_depth_intersection(self):
        lo,hi=forbidden_height((10,20),(-3,5))
        for receiver in np.linspace(10,20,11):
            for body in np.linspace(-3,5,11):
                height=SIN*(receiver-body)
                self.assertGreaterEqual(height,lo+MARGIN_Z-1e-12)
                self.assertLessEqual(height,hi-MARGIN_Z+1e-12)

    def test_free_bands_exclude_receiver_intervals(self):
        self.assertEqual(free_bands([[-20,10],[15,30],[200,300]]),[[10,15],[30,200]])

    def test_disconnected_temporal_bands_fail(self):
        self.assertEqual(band_paths([[[5,10]],[[20,30]]],np.array([8,25])),[])

    def test_entire_cubic_stays_in_time_varying_free_bands(self):
        n=12;dt=.125;lo=10+np.sin(np.arange(n)*2*np.pi/n)
        bands=[[[float(x),float(x+4)]]for x in lo]
        result=solve_bands(bands,[0]*n,np.full(n,9.),dt)
        self.assertTrue(result['success'])
        curve=CubicSpline(result['curve']['times'],result['curve']['heights'],bc_type='periodic')
        for i,band in enumerate(bands):
            values=curve(np.linspace(i*dt,(i+1)*dt,101))
            self.assertGreaterEqual(values.min(),band[0][0]-1e-7)
            self.assertLessEqual(values.max(),band[0][1]+1e-7)
        self.assertLessEqual(result['maximum_speed'],7+1e-6)
        self.assertLessEqual(result['maximum_acceleration'],40+1e-6)

if __name__=='__main__':unittest.main()
