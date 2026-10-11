"""Double-precision fallback for a ray landing on a triangulated surface edge."""
import numpy as np

def boundary_hits(origin,direction,meshes):
 """Return inclusive-edge intersections; never extend beyond 1e-7 barycentric."""
 o=np.asarray(origin,dtype=np.float64);d=np.asarray(direction,dtype=np.float64);hits=[]
 for owner,(vertices,triangles) in meshes.items():
  v=np.asarray(vertices,dtype=np.float64);t=np.asarray(triangles,dtype=int);a=v[t[:,0]];e1=v[t[:,1]]-a;e2=v[t[:,2]]-a
  p=np.cross(np.broadcast_to(d,e2.shape),e2);det=np.einsum('ij,ij->i',e1,p);valid=np.abs(det)>1e-12
  inv=np.zeros_like(det);inv[valid]=1/det[valid];s=o-a;u=np.einsum('ij,ij->i',s,p)*inv;q=np.cross(s,e1);w=q@d*inv;distance=np.einsum('ij,ij->i',e2,q)*inv
  selected=valid&(u>=-1e-7)&(w>=-1e-7)&(u+w<=1+1e-7)&(distance>=0)
  for i in np.flatnonzero(selected):hits.append({'distance':float(distance[i]),'owner':owner,'triangle':int(i),'barycentric':[float(1-u[i]-w[i]),float(u[i]),float(w[i])]})
 return sorted(hits,key=lambda x:x['distance'])
