"""Regression: white RGBA mask RGB must not erase the protected domain."""
import unittest
import numpy as np
from PIL import Image
from restart2_texture_bake_receipt import verify_protected_rgba

class ProtectionTests(unittest.TestCase):
    def setUp(self):
        self.source=np.zeros((2,2,4),dtype=np.uint8)
        self.source[:,:,3]=255
        rgba=np.full((2,2,4),255,dtype=np.uint8);rgba[0,0,3]=0
        self.mask=Image.fromarray(rgba)

    def test_editable_changes_allowed(self):
        result=self.source.copy();result[0,0]=[40,50,60,255]
        self.assertEqual(verify_protected_rgba(self.source,result,self.mask),1)

    def test_protected_rgb_and_alpha_changes_rejected(self):
        for channel in range(4):
            result=self.source.copy();result[1,1,channel]^=1
            with self.assertRaisesRegex(ValueError,'Protected RGBA bytes differ'):
                verify_protected_rgba(self.source,result,self.mask)

    def test_empty_protected_domain_rejected(self):
        with self.assertRaisesRegex(ValueError,'both editable and protected'):
            verify_protected_rgba(self.source,self.source,Image.new('RGBA',(2,2),(255,255,255,0)))

if __name__=='__main__':unittest.main()
