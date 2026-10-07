import unittest,tempfile,hashlib
from pathlib import Path
import numpy as np
from scipy.interpolate import CubicSpline
from restart14_butterfly07_anatomy_contacts import clip_planes,triangle_planes,validate_rest_shape,geometry_binding,curve_acceleration_bound,WING

class AnatomyContactTests(unittest.TestCase):
    def test_changed_rest_shape_cannot_reuse_old_geometry_audit(self):
        fit={"fixed_geometry":{"wing_outline":WING.tolist(),"body_radii":[.4,2.7,.45],"wing_hinge_offsets":[-.22,.22]}}
        validate_rest_shape(fit)
        fit["fixed_geometry"]["wing_outline"][1][0]+=.1
        with self.assertRaises(AssertionError):validate_rest_shape(fit)

    def test_explicit_geometry_binding_requires_hash_and_matching_shape(self):
        with tempfile.TemporaryDirectory() as directory:
            helper=Path(directory)/'shape.py'
            helper.write_text("def fixed_geometry():\n return dict(wing_outline=[[0,0,0],[3,4,0],[0,2,0]],body_radii=[1,2,1],wing_hinge_offsets=[-.5,.5])\ndef geometry(p):\n return p\n")
            digest=hashlib.sha256(helper.read_bytes()).hexdigest()
            fit={'fixed_geometry':dict(wing_outline=[[0,0,0],[3,4,0],[0,2,0]],body_radii=[1,2,1],wing_hinge_offsets=[-.5,.5])}
            function,radii,binding=geometry_binding(fit,helper,digest)
            self.assertEqual(radii,{'body':2.,'wing':5.5})
            self.assertEqual(binding['sha256'],digest)
            self.assertEqual(function([1]),[1])
            with self.assertRaises(AssertionError):geometry_binding(fit,helper,'0'*64)
            fit['fixed_geometry']['body_radii'][0]=3
            with self.assertRaises(AssertionError):geometry_binding(fit,helper,digest)

    def test_curve_bound_includes_interior_spline_knots(self):
        curve=CubicSpline([0,.25,.5,.75,1],[0,1,-1,1,0],bc_type='natural')
        bound=curve_acceleration_bound(curve,0,1)
        self.assertGreater(bound,max(abs(curve(0,2)),abs(curve(1,2))))
        self.assertGreaterEqual(bound,np.abs(curve(np.linspace(0,1,1001),2)).max()-1e-10)

    def test_crossing_receiver_triangle_reaches_zero_thickness_wing(self):
        wing=np.array([[0.,0.,0.],[2.,0.,0.],[0.,2.,0.]])
        receiver=np.array([[.5,.5,-1.,0.,0.],[.5,.5,1.,1.,0.],[1.,.5,1.,0.,1.]])
        piece=clip_planes(receiver,triangle_planes(wing))
        self.assertGreater(len(piece),0)
        self.assertLess(abs(piece[:,2]).max(),2e-8)
        self.assertTrue(np.all(piece[:,0]+piece[:,1]<2))

    def test_receiver_outside_wing_edges_is_rejected(self):
        wing=np.array([[0.,0.,0.],[2.,0.,0.],[0.,2.,0.]])
        receiver=np.array([[4.,4.,-1.,0.,0.],[4.,4.,1.,1.,0.],[5.,4.,1.,0.,1.]])
        self.assertEqual(len(clip_planes(receiver,triangle_planes(wing))),0)

if __name__=='__main__':unittest.main()
