"""Inferred branch-scale crown clumps anchored to this tree's source supports."""
import math
import numpy as np


class CrownClumps:
    def __init__(self, packet, center, sin, cos, ray):
        fx,fy,fw,fh=packet['bbox']
        self.center=np.asarray(center);self.cx=int(fx+fw/2);self.cy=fy+fh/2
        self.sin=sin;self.cos=cos;self.ray=np.asarray(ray)
        self.centers=[];self.radii=[];self.fallbacks=0
        for index,lobe in enumerate(packet['branch_supports']):
            sx,sy=lobe['seed'];left,top,right,bottom=lobe['bbox']
            depth=fw*.34*math.sin(index*2.399+1.1)
            self.centers.append(self.point(sx,sy,depth))
            self.radii.append([max(24,(right-left)*.43),fw*(.23+.035*math.cos(index*1.7)),
                               max(28,(bottom-top)*.42/cos)])
        self.centers=np.asarray(self.centers);self.radii=np.asarray(self.radii)
        self.weights=np.prod(self.radii,axis=1);self.weights/=self.weights.sum()

    def point(self,x,y,depth=0):
        return self.center+np.array([x-self.cx,-(y-self.cy)*self.sin,-(y-self.cy)*self.cos])+self.ray*depth

    def front_depth(self,x,y):
        origin=self.point(x,y)
        relative=origin-self.centers
        a=np.sum((self.ray/self.radii)**2,axis=1)
        b=2*np.sum(relative*self.ray/self.radii**2,axis=1)
        c=np.sum((relative/self.radii)**2,axis=1)-1
        disc=b*b-4*a*c
        good=disc>=0
        if good.any():
            values=np.where(good,(-b+np.sqrt(np.maximum(0,disc)))/(2*a),-np.inf)
            depth=float(np.max(values))
        else:
            # Pixels beyond an inferred clump support keep their native ray;
            # the nearest branch supplies depth, never a common globe shell.
            self.fallbacks+=1
            closest=int(np.argmin(c-b*b/(4*a)))
            depth=float(-b[closest]/(2*a[closest]))
        return depth+12*math.sin(x*.041+y*.027)+8*math.sin(x*.083-y*.063)

    def sample(self,rng):
        index=int(rng.choice(len(self.centers),p=self.weights))
        unit=rng.normal(size=3);unit/=np.linalg.norm(unit)
        unit*=rng.uniform(.025,1.)**(1/3)
        angle=math.atan2(unit[1],unit[0])
        irregular=1+.12*math.sin(5*angle+index)+.07*math.cos(9*angle-index)
        return self.centers[index]+unit*self.radii[index]*irregular

    def report(self):
        return dict(centers=self.centers.tolist(),radii=self.radii.tolist(),
                    source_supports=len(self.centers),nearest_branch_fallbacks=self.fallbacks,
                    inference='Native artwork supports seed separate branch-scale clumps; all hidden depths are inferred.')
