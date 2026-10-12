"""Local exposed-face depth proposal and full triangulated rear volume for bank53."""
import argparse,copy,hashlib,json,math,sys
from pathlib import Path
from collections import Counter
import numpy as np
from scipy.spatial import Delaunay
from scipy.sparse import lil_matrix
from scipy.sparse.linalg import spsolve
S=math.sin(math.radians(35));C=math.cos(math.radians(35))
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def cross(a,b):return a[0]*b[1]-a[1]*b[0]
def inside(q,poly):
 odd=False
 for a,b in zip(poly,poly[1:]+poly[:1]):
  if (a[1]>q[1])!=(b[1]>q[1]) and q[0]<(b[0]-a[0])*(q[1]-a[1])/(b[1]-a[1])+a[0]:odd=not odd
 return odd

def main():
 p=argparse.ArgumentParser();p.add_argument('--input',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args();assert not a.output.exists();old=json.loads(a.input.read_text());data=copy.deepcopy(old);v=np.array(data['vertices']);oldv=v.copy();faces=data['faces'];nfront=data['front_vertex_count'];oldcenter=len(v)-1;groundids=set(map(int,data['ground_footprint_vertex_ids'].values()));rear_start=nfront;rear_end=min(groundids-{0,max(i for i in groundids if i<nfront)})
 # Source positions stay exact. The preview cap height is not a global geometric authority.
 for i in range(nfront):
  if i in groundids:continue
  x=v[i,0];y=-v[i,1]*S-v[i,2]*C
  uplift=22*math.exp(-1*((x-177)**2/40**2+(y-286)**2/20**2))+6*math.exp(-1*((x-186)**2/32**2+(y-316)**2/15**2))
  v[i,1]-=uplift/S;v[i,2]+=uplift/C
 # Keep the shoulder displacement relative to its original corresponding front point.
 for i in range(rear_start,rear_end):
  matches=np.flatnonzero(np.linalg.norm(oldv[:nfront][:,[0,2]]-oldv[i,[0,2]],axis=1)<1e-7);assert len(matches)==1,(i,matches)
  j=int(matches[0]);v[i]+=v[j]-oldv[j]
  z=v[i,2];v[i,1]+=16*math.sin(math.pi*min(1.,z/85))**2
 # Replace the single-point rear fan with a constrained disk, retaining its entire boundary.
 rearfaces=[f for f in faces if oldcenter in f];boundary_edges=[]
 for f in rearfaces:
  j=f.index(oldcenter);boundary_edges.append((f[(j+1)%3],f[(j+2)%3]))
 links={x:y for x,y in boundary_edges};loop=[boundary_edges[0][0]]
 while links[loop[-1]]!=loop[0]:loop.append(links[loop[-1]])
 assert len(loop)==len(boundary_edges)
 boundary=v[loop];chart=np.column_stack((boundary[:,0],boundary[:,2]-.2*boundary[:,1]));poly=chart.tolist()
 # The chart must be a simple boundary, not a folded fan.
 def crosses(i,j,k,l,pts):
  aa,bb,cc,dd=pts[[i,j,k,l]];return cross(bb-aa,cc-aa)*cross(bb-aa,dd-aa)<-1e-9 and cross(dd-cc,aa-cc)*cross(dd-cc,bb-cc)<-1e-9
 for i in range(len(loop)):
  for j in range(i+1,len(loop)):
   if len({i,(i+1)%len(loop),j,(j+1)%len(loop)})<4:continue
   assert not crosses(i,(i+1)%len(loop),j,(j+1)%len(loop),chart),('Folded rear chart',i,j)
 points=poly[:]
 def distance(q):
  ans=1e9
  for x,y in zip(chart,np.roll(chart,-1,axis=0)):
   d=y-x;t=np.clip(np.dot(q-x,d)/np.dot(d,d),0,1);ans=min(ans,float(np.linalg.norm(q-x-t*d)))
  return ans
 for yy in np.arange(chart[:,1].min()+2,chart[:,1].max(),4):
  for xx in np.arange(chart[:,0].min()+2,chart[:,0].max(),4):
   q=[float(xx),float(yy)]
   if inside(q,poly) and distance(np.array(q))>.1:points.append(q)
 pts=np.array(points);tris=Delaunay(pts).simplices.tolist();locked=set()
 for i in range(len(loop)):
  j=(i+1)%len(loop);target=tuple(sorted((i,j)))
  for attempt in range(200):
   adj={}
   for fi,f in enumerate(tris):
    for x,y in zip(f,f[1:]+f[:1]):adj.setdefault(tuple(sorted((x,y))),[]).append(fi)
   if target in adj:locked.add(target);break
   choices=[]
   for (x,y),pair in adj.items():
    if len(pair)!=2 or (x,y) in locked or not crosses(i,j,x,y,pts):continue
    fa,fb=pair;ca=next(k for k in tris[fa] if k not in (x,y));cb=next(k for k in tris[fb] if k not in (x,y))
    if crosses(x,y,ca,cb,pts):choices.append((crosses(i,j,ca,cb,pts),fa,fb,ca,cb,x,y))
   assert choices,('Cannot recover rear boundary',i,j)
   _,fa,fb,ca,cb,x,y=sorted(choices)[0];tris[fa]=[ca,cb,x];tris[fb]=[cb,ca,y]
  else:raise RuntimeError('Rear boundary recovery did not converge')
 tris=[f for f in tris if inside(pts[f].mean(0),poly)];neighbor=[set()for _ in pts]
 for f in tris:
  for x,y in zip(f,f[1:]+f[:1]):neighbor[x].add(y);neighbor[y].add(x)
 n=len(loop);mat=lil_matrix((len(pts)-n,len(pts)-n));rhs=np.zeros(len(pts)-n)
 for i in range(n,len(pts)):
  mat[i-n,i-n]=len(neighbor[i])
  for j in neighbor[i]:
   if j<n:rhs[i-n]+=boundary[j,1]
   else:mat[i-n,j-n]-=1
 values=spsolve(mat.tocsr(),rhs);bulge=spsolve(mat.tocsr(),np.ones(len(pts)-n));bulge/=bulge.max();newverts=v[:-1].tolist();mapping=loop[:]
 for q,y,b in zip(pts[n:],values,bulge):
  y+=25*b;y=max(y,(.3-q[1])/.2);z=q[1]+.2*y;mapping.append(len(newverts));newverts.append([q[0],float(y),float(z)])
 faces=[f for f in faces if oldcenter not in f]+[[mapping[i]for i in f]for f in tris]
 # Orient and validate the entire resulting enclosure.
 edges={}
 for i,f in enumerate(faces):
  for x,y in zip(f,f[1:]+f[:1]):edges.setdefault(tuple(sorted((x,y))),[]).append((i,x,y))
 assert set(map(len,edges.values()))=={2}
 flips={0:False};stack=[0]
 while stack:
  i=stack.pop()
  for x,y in zip(faces[i],faces[i][1:]+faces[i][:1]):
   for j,u,w in edges[tuple(sorted((x,y)))]:
    if j==i:continue
    value=flips[i]^(u==x)
    if j in flips:assert flips[j]==value
    else:flips[j]=value;stack.append(j)
 assert len(flips)==len(faces);faces=[f[::-1]if flips[i]else f for i,f in enumerate(faces)];vv=np.array(newverts);vol=sum(np.dot(vv[x],np.cross(vv[y],vv[z]))/6 for x,y,z in faces)
 if vol<0:faces=[f[::-1]for f in faces];vol=-vol
 areas=[np.linalg.norm(np.cross(vv[y]-vv[x],vv[z]-vv[x]))/2 for x,y,z in faces];assert min(areas)>1e-8
 for i in groundids:assert np.array_equal(vv[i],oldv[i])
 errors=[]
 for t in data['traces']:
  for i,q in zip(t['vertices'],t['source_xy']):errors.append(max(abs(vv[i,0]-q[0]),abs(-vv[i,1]*S-vv[i,2]*C-q[1])))
 assert max(errors)<1e-9
 data.update(vertices=newverts,faces=faces,signed_volume=float(vol),minimum_triangle_area=float(min(areas)),euler=len(vv)-len(edges)+len(faces),status='HOLD_FULLER_REAR_LOCAL_FACE_DEPTH_COMBINED_GUARDS_REQUIRED')
 data.pop('authored_height_cap',None)
 data['local_bulk_revision']={'input':str(a.input.resolve()),'sha256':sha(a.input),'old_world_vertices':oldv.tolist(),'old_faces':old['faces'],'front_projection_max_error':max(errors),'new_rear_vertices':len(pts)-n,'native_height_range':[float(vv[:nfront,2].min()*C),float(vv[:nfront,2].max()*C)],'ground_vertices_exact':True,'limits':['Local A/B exposed-face uplifts are inferred candidates, not approved ownership.','No uniform62 heightcap; original topback remains separate from projecting faces.','Rear harmonic disk replaces singlepoint fan and increases enclosing bulk; strict intersections/contacts require independent checks.']};a.output.mkdir();(a.output/'geometry.json').write_text(json.dumps(data,indent=2)+'\n');print({k:data[k]for k in ['status','signed_volume','minimum_triangle_area','euler']})
if __name__=='__main__':main()
