"""Source-preserving upper boundary with an inferred smooth local graft transition."""
import math
import bpy,bmesh,numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree

def blend(obj):
 mesh=obj.data;points=np.array([v.co[:]for v in mesh.vertices]);center=points[(points[:,2]>98)&(points[:,2]<106),:2].mean(0);tree=BVHTree.FromPolygons([v.co for v in mesh.vertices],[list(f.vertices)for f in mesh.polygons]);rings=[]
 for z in [88,110]:
  radii=[]
  for angle in np.arange(256)*2*math.pi/256:
   d=Vector((math.cos(angle),math.sin(angle),0));hit,n,i,dist=tree.ray_cast(Vector((*center,z))+d*120,-d,120);assert hit is not None,(z,angle);radii.append(np.linalg.norm(np.array(hit[:2])-center))
  rings.append(radii)
 before=points.copy()
 for i,p in enumerate(points):
  z=p[2]
  if not 88<z<110:continue
  d=p[:2]-center;r=np.linalg.norm(d);angle=(math.atan2(d[1],d[0])%(2*math.pi))*256/(2*math.pi);a=int(angle);b=(a+1)%256;t=angle-a;u=(z-88)/22;radius=(1-u)*((1-t)*rings[0][a]+t*rings[0][b])+u*((1-t)*rings[1][a]+t*rings[1][b]);weight=min(1,(z-88)/12,(110-z)/6);weight=weight*weight*(3-2*weight);points[i,:2]=center+d*((1-weight)+weight*radius/r)
 for v,p in zip(mesh.vertices,points):v.co=p
 bm=bmesh.new();bm.from_mesh(mesh);verts=[v for v in bm.verts if 88<v.co.z<110];bmesh.ops.remove_doubles(bm,verts=verts,dist=.0001);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));topology=dict(nonmanifold=sum(not e.is_manifold for e in bm.edges),degenerate=sum(f.calc_area()<1e-9 for f in bm.faces));bm.to_mesh(mesh);bm.free();mesh.update();mesh.normals_split_custom_set([(0,0,0)]*len(mesh.loops))
 return dict(max_movement=float(np.linalg.norm(points-before,axis=1).max()),topology=topology)
