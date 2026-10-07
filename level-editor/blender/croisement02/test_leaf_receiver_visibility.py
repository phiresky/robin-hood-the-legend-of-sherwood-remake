"""Geometric visibility cases independent of the map source fixtures."""
import unittest
import numpy as np
from leaf_receiver_visibility import frontmost

def candidate(name,polygon,z):
    return dict(owner=name,polygon=[np.array(p,dtype=float)for p in polygon],mapping=np.array([[1,0,z[0]],[0,1,z[1]],[0,0,z[2]]],float))

def surface_area(rows,owner=None):
    total=0
    for row in rows:
        if owner is not None and row['owner']!=owner:continue
        p=row['points'];total+=abs(sum(a[0]*b[1]-b[0]*a[1]for a,b in zip(p,p[1:]+p[:1])))/2
    return total

class VisibilityTests(unittest.TestCase):
    def test_hidden_parallel_face_removed(self):
        tri=[(0,0),(2,0),(0,2)]
        rows=frontmost([candidate('back',tri,(0,0,0)),candidate('front',tri,(0,0,1))],(0,0,1))
        self.assertAlmostEqual(surface_area(rows,'back'),0)
        self.assertAlmostEqual(surface_area(rows,'front'),2)

    def test_partial_cover_preserves_visible_remainder(self):
        rows=frontmost([candidate('base',[(0,0),(2,0),(0,2)],(0,0,0)),candidate('cap',[(0,0),(1,0),(0,1)],(0,0,2))],(0,0,1))
        self.assertAlmostEqual(surface_area(rows,'base'),1.5)
        self.assertAlmostEqual(surface_area(rows,'cap'),.5)

    def test_crossing_planes_split_at_depth_equality(self):
        tri=[(0,0),(2,0),(0,2)]
        rows=frontmost([candidate('rising',tri,(1,0,0)),candidate('falling',tri,(-1,0,1))],(0,0,1))
        self.assertAlmostEqual(surface_area(rows),2,places=5)
        for row in rows:
            center=np.mean(row['points'],axis=0)
            if row['owner']=='rising':self.assertGreater(center[0],.5)
            else:self.assertLess(center[0],.5)
            for x,y,z in row['points']:self.assertAlmostEqual(z,x if row['owner']=='rising'else 1-x)

if __name__=='__main__':unittest.main()
