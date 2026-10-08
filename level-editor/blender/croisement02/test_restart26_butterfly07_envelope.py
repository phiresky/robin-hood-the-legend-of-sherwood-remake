import unittest
import numpy as np
from scipy.interpolate import CubicSpline
from restart26_butterfly07_clearance_envelope import forbidden_height,MARGIN_Z
from restart26_butterfly07_envelope_path import free_bands,band_paths,solve_bands
from restart14_butterfly_canopy22_audit import SIN
from restart26_butterfly07_piece_envelope import translation_interval,merge_bands,body_box
from scipy.spatial import ConvexHull
from scipy.spatial.transform import Rotation,Slerp
from restart21_butterfly07_geometry_v2 import geometry,fixed_geometry
from restart26_butterfly07_global_corridor import choose_bands

class ContinuousEnvelopeTests(unittest.TestCase):
    def test_global_bands_reject_nearby_but_unreachable_branch(self):
        bands=[[[5.+8*(i%2),6.+8*(i%2)],[30.,35.]]for i in range(8)]
        baseline=np.array([5.+8*(i%2)for i in range(8)])
        result=choose_bands(bands,baseline,.125,seconds=10)
        self.assertTrue(result['success'])
        self.assertEqual(result['path'],[1]*8)
        self.assertTrue(solve_bands(bands,result['path'],baseline,.125)['success'])

    def test_body_box_sweep_contains_actual_interpolated_body(self):
        radii=np.array(fixed_geometry()['body_radii'])
        start=np.array([30.,-70.,110.]);end=np.array([-55.,20.,-140.])
        rotations=Rotation.from_euler('xyz',[start,end],degrees=True)
        slerp=Slerp([0,1],rotations)
        angle=(rotations[0].inv()*rotations[1]).magnitude()
        for i in range(8):
            def parameters(t):
                return np.r_[slerp([t])[0].as_euler('xyz',degrees=True),0.,0.,0.,0.]
            points=np.vstack([body_box(parameters(t),radii) for t in [i/8,(i+1)/8]])
            planes=ConvexHull(points).equations
            pad=max(radii)*angle**2/(8*8**2)+1e-6
            for t in np.linspace(i/8,(i+1)/8,33):
                body,_=geometry(parameters(t))
                self.assertLessEqual(float((body@planes[:,:3].T+planes[:,3]).max()),pad)

    def test_minkowski_depth_band_matches_cube_receiver_contact(self):
        cube=np.array([[x,y,z]for x in [-1.,1.]for y in [-1.,1.]for z in [-1.,1.]])
        receiver=np.array([[-3.,-3.,5.],[3.,-3.,5.],[0.,3.,5.]])
        band=translation_interval(receiver,cube,1e-6)
        np.testing.assert_allclose(band,[4*SIN-MARGIN_Z,6*SIN+MARGIN_Z],atol=1e-5)
        self.assertIsNone(translation_interval(receiver+np.array([20,0,0]),cube,1e-6))

    def test_interval_union_preserves_real_depth_gaps(self):
        self.assertEqual(merge_bands([[1,2],[1.5,3],[4,5]]),[[1.,3.],[4.,5.]])

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
