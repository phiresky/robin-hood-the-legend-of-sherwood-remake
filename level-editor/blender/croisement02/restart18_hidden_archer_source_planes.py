"""Exact observed leaf planes for source-space inferred geometry constraints."""
import math
import numpy as np
from restart18_hidden_archer_compact_graph import unpack
from restart18_hidden_archer_route_cpu import RAY,SIN,screen,RockDepth
class SourcePlanes:
    def __init__(self,state):
        a={k:unpack(v) for k,v in state['arrays'].items()};v=a['world_vertices'];loops=a['polygon_loop_vertices'];offset=a['polygon_offsets'];self.planes={}
        for i in range(state['native_pixels']):
            poly=v[loops[offset[2*i]:offset[2*i+1]]].astype(float);n=np.cross(poly[1]-poly[0],poly[2]-poly[0]);n/=np.linalg.norm(n);pixel=tuple(np.floor(screen(poly.mean(0))).astype(int));self.planes[pixel]=(n,float(n@poly[0]))
    def depth(self,point):
        plane=self.planes.get(tuple(np.floor(point).astype(int)))
        if plane is None:return None
        n,d=plane;start=np.array([point[0],-point[1]/SIN,0.]);return float(start@RAY+(d-n@start)/(n@RAY))
    def allowed_depth(self,point,radius=.08):
        # Include every leaf plane whose native cell intersects the footprint.
        # Extrapolate each such plane to the center; subtracting a slope bound
        # accounts for every position within the projected radius.
        best=math.inf
        for y in range(math.floor(point[1]-radius),math.floor(point[1]+radius)+1):
            for x in range(math.floor(point[0]-radius),math.floor(point[0]+radius)+1):
                plane=self.planes.get((x,y))
                if plane is None:continue
                n,d=plane;start=np.array([point[0],-point[1]/SIN,0.]);value=float(start@RAY+(d-n@start)/(n@RAY));gradient=np.array([-n[0]/(n@RAY),(RAY[1]*-1/SIN)+(n[1]/SIN)/(n@RAY)]);best=min(best,value-np.linalg.norm(gradient)*radius)
        return best
    def triangles_clear(self,tri,subdivisions=4):
        xy=screen(tri);lo=np.floor(xy.min((0,1))).astype(int);hi=np.floor(xy.max((0,1))).astype(int);offsets=(np.arange(subdivisions)+.5)/subdivisions;points=np.array([(x+dx,y+dy) for y in range(max(0,lo[1]),hi[1]+1) for x in range(lo[0],hi[0]+1) for dy in offsets for dx in offsets])
        if not len(points):return True
        front=RockDepth(tri).front(points)
        for point,value in zip(points,front):
            if not np.isfinite(value):continue
            old=self.depth(point)
            if old is None or value>old-1e-4:return False
        return True
    def triangle_source_conflicts(self,tri):
        """Clip projected triangles to native cells; compare both linear planes."""
        conflicts=[]
        for t in tri:
            xy=screen(t);depth=t@RAY;poly=np.column_stack([xy,depth]);lower=np.floor(xy.min(0)).astype(int);upper=np.floor(xy.max(0)).astype(int)
            for y in range(max(0,lower[1]),upper[1]+1):
                for x in range(lower[0],upper[0]+1):
                    clipped=poly.copy()
                    for axis,value,sign in [(0,x,1),(0,x+1,-1),(1,y,1),(1,y+1,-1)]:
                        if not len(clipped):break
                        out=[]
                        for a,b in zip(clipped,np.roll(clipped,-1,axis=0)):
                            da=(a[axis]-value)*sign;db=(b[axis]-value)*sign;ina=da>=0;inb=db>=0
                            if ina:out.append(a)
                            if ina!=inb:out.append(a+(b-a)*da/(da-db))
                        clipped=np.array(out)
                    if len(clipped)<3:continue
                    a=clipped[1:-1,:2]-clipped[0,:2];b=clipped[2:,:2]-clipped[0,:2];area=abs(np.sum(a[:,0]*b[:,1]-a[:,1]*b[:,0]))/2
                    if area<1e-10:continue
                    plane=self.planes.get((x,y))
                    if plane is None:conflicts.append(dict(pixel=[x,y],role='native-empty',projected_triangle_area=float(area)));continue
                    n,d=plane;starts=np.column_stack([clipped[:,0],-clipped[:,1]/SIN,np.zeros(len(clipped))]);old=starts@RAY+(d-starts@n)/(n@RAY);excess=float(np.max(clipped[:,2]-old))
                    if excess>=-.0001:conflicts.append(dict(pixel=[x,y],role='known-native',maximum_depth_excess=excess,projected_triangle_area=float(area)))
        return conflicts
    def triangles_clear_exact(self,tri):return not self.triangle_source_conflicts(tri)
