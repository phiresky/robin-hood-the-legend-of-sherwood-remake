"""Changing inferred crown depth must never move its native source rays."""
import math
import unittest
import numpy as np
from crown_clumps import CrownClumps


class ClumpTests(unittest.TestCase):
    def setUp(self):
        self.sin=math.sin(math.radians(35));self.cos=math.cos(math.radians(35))
        ray=np.array([0,-self.cos,self.sin])
        self.packet=dict(bbox=[0,0,200,260],branch_supports=[
            dict(seed=[55,60],bbox=[10,15,100,105]),
            dict(seed=[150,100],bbox=[95,45,205,155]),
            dict(seed=[95,205],bbox=[35,145,155,265])])
        self.clumps=CrownClumps(self.packet,[100,-500/self.sin,(500-130)/self.cos],self.sin,self.cos,ray)

    def test_each_inferred_branch_center_keeps_its_source_seed(self):
        for center,support in zip(self.clumps.centers,self.packet['branch_supports']):
            np.testing.assert_allclose([center[0],-center[1]*self.sin-center[2]*self.cos],support['seed'],atol=1e-10)

    def test_fragment_depth_never_moves_source_projection(self):
        for x,y in [(10,10),(55,60),(150,100),(190,250),(-30,300)]:
            depth=self.clumps.front_depth(x,y)
            point=self.clumps.point(x,y,depth)
            self.assertTrue(math.isfinite(depth))
            np.testing.assert_allclose([point[0],-point[1]*self.sin-point[2]*self.cos],[x,y],atol=1e-10)


if __name__=='__main__':unittest.main()
