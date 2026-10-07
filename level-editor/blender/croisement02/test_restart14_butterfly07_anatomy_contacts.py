import unittest
import numpy as np
from restart14_butterfly07_anatomy_contacts import clip_planes,triangle_planes

class AnatomyContactTests(unittest.TestCase):
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
