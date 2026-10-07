import unittest
import numpy as np
from restart14_butterfly07_prism_contacts import box_clip,bilinear_maximum,alpha_maximum


class ContactTests(unittest.TestCase):
    def test_disjoint_depth_interval_is_empty(self):
        triangle=np.array([[0.,0.,20.,0.,0.],[1.,0.,21.,1.,0.],[0.,1.,22.,0.,1.]])
        self.assertEqual(len(box_clip(triangle,[(2,10.,19.)])),0)

    def test_clip_keeps_affine_attributes(self):
        triangle=np.array([[0.,0.,0.,0.,0.],[2.,0.,2.,1.,0.],[0.,2.,4.,0.,1.]])
        piece=box_clip(triangle,[(0,.5,1.),(1,.25,.75),(2,1.,2.)])
        self.assertGreater(len(piece),0)
        np.testing.assert_allclose(piece[:,2],piece[:,0]+2*piece[:,1])
        np.testing.assert_allclose(piece[:,3:5],piece[:,:2]/2)
        self.assertTrue(np.all((piece[:,2]>=1)&(piece[:,2]<=2)))

    def test_zero_width_height_slice_retains_line(self):
        triangle=np.array([[0.,0.,0.,0.,0.],[2.,0.,2.,1.,0.],[0.,2.,4.,0.,1.]])
        piece=box_clip(triangle,[(2,1.3,1.3)])
        self.assertGreaterEqual(len(piece),2)
        np.testing.assert_array_equal(piece[:,2],np.full(len(piece),1.3))
        np.testing.assert_allclose(piece[:,2],piece[:,0]+2*piece[:,1])

    def test_alpha_maximum_finds_edge_interior_not_just_vertices(self):
        # Checker alpha along x=y is 2t(1-t); both endpoint samples are zero.
        poly=np.array([[0.,0.],[1.,1.]])
        value,point=bilinear_maximum(poly,[0.,1.,1.,0.],0,1)
        self.assertAlmostEqual(value,.5)
        np.testing.assert_allclose(point,[.5,.5])

    def test_continuous_alpha_contact_and_transparent_rejection(self):
        image=np.zeros((2,2,4),dtype=np.uint8);image[0,1,3]=255;image[1,0,3]=255
        poly=np.array([[0.,0.,3.,.25,.25],[1.,1.,3.,.75,.75]])
        value,point,cells=alpha_maximum(poly,image,{'wrapS':33071,'wrapT':33071},1.)
        self.assertAlmostEqual(value,.5);self.assertGreater(cells,0)
        np.testing.assert_allclose(point[3:5],[.5,.5])
        image[:,:,3]=0
        self.assertEqual(alpha_maximum(poly,image,{},1.)[0],0.)


if __name__=='__main__':unittest.main()
