"""Reconstruct complete visible boulder fronts, retaining closed boundary/contact bulk."""
import json,math,hashlib
from collections import Counter
from pathlib import Path
import numpy as np
from scipy.spatial import Delaunay
from scipy.interpolate import LinearNDInterpolator
from scipy.sparse import lil_matrix
from scipy.sparse.linalg import spsolve
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/active/croisement03/restart2';O=B/'bank-boulder-fronts-cpu-v8';E=B/'bank-boulder-east-cpu-v8';S=math.sin(math.radians(35));C=math.cos(math.radians(35));RAY=np.array([0.,-C,S])
def project(v):return np.column_stack((v[:,0],-v[:,1]*S-v[:,2]*C))
def inside(p,poly):
 x,y=p;hit=False
 for a,b in zip(poly,poly[1:]+poly[:1]):
  if (a[1]>y)!=(b[1]>y) and x<(b[0]-a[0])*(y-a[1])/(b[1]-a[1])+a[0]:hit=not hit
 return hit

def rebuild(mesh,owner,proposals,trace_rows):
 old=np.array(mesh['vertices']);nr=len(mesh['ring_names']);n=len(old)//nr;nf=n-3;assert len(old)==nr*n
 front_ids=[r*n+i for r in range(nr)for i in range(nf)];xy=project(old);interp=LinearNDInterpolator(xy[front_ids],old[front_ids,2]);boundary_ids=list(range(nf))+[r*n+nf-1 for r in range(1,nr)]+[(nr-1)*n+i for i in range(nf-2,-1,-1)]+[r*n for r in range(nr-2,0,-1)];polygon=[xy[i].tolist()for i in boundary_ids];border=set(boundary_ids)
 # Preserve all side/back/bottom faces. Only front strips are reconstructed.
 removed={r*n*2+i*2+j for r in range(nr-1)for i in range(nf-1)for j in (0,1)};kept=[list(f)for i,f in enumerate(mesh['faces'])if i not in removed];points=[p.tolist()for p in xy[front_ids]];source_ids=list(front_ids);lookup={tuple(np.round(p,8)):i for i,p in enumerate(points)};local={j:i for i,j in enumerate(front_ids)};bound_edges=[(local[a],local[b])for a,b in zip(boundary_ids,boundary_ids[1:]+boundary_ids[:1])];edges=list(bound_edges);world=[p.tolist()for p in old];fixed={local[i]:old[i].copy()for i in border}
 def add(p):
  key=tuple(np.round(p,8))
  if key in lookup:return lookup[key]
  index=len(points);lookup[key]=index;points.append(list(p));source_ids.append(len(world));world.append([0.,0.,0.]);return index
 trace_local=[]
 for trace in trace_rows:
  ids=[add(p)for p in trace['source']];trace_local.append((trace['id'],ids));edges.extend(zip(ids,ids[1:]))
 for face in proposals:
  poly=face['native_outline'];lo=np.floor(np.min(poly,axis=0)).astype(int);hi=np.ceil(np.max(poly,axis=0)).astype(int)
  for y in range(lo[1],hi[1]+1,2):
   for x in range(lo[0],hi[0]+1,2):
    if inside([x,y],poly)and inside([x,y],polygon):add([x,y])
  for a,b in zip(poly,poly[1:]+poly[:1]):
   for t in np.linspace(0,1,max(2,int(math.dist(a,b))+1)):
    p=np.array(a)*(1-t)+np.array(b)*t
    if inside(p,polygon):add(p)
 # Recover perimeter and native sample paths. Splits on the perimeter also
 # split the retained neighboring triangles, preventing boundary T-junctions.
 def split_kept(ga,gb,gm):
  out=[];count=0
  for f in kept:
   found=False
   for j in range(3):
    a,b,c=f[j],f[(j+1)%3],f[(j+2)%3]
    if {a,b}=={ga,gb}:out.extend([[a,gm,c],[gm,b,c]]);found=True;count+=1;break
   if not found:out.append(f)
  assert count==1,(ga,gb,count)
  kept[:]=out
 boundary={tuple(sorted(e))for e in bound_edges}
 for iteration in range(20):
  noded=[]
  for a,b in edges:
   pa=np.array(points[a]);v=np.array(points[b])-pa;l2=np.dot(v,v);nodes=[]
   for k,p in enumerate(points):
    if k in (a,b):continue
    t=float(np.dot(np.array(p)-pa,v)/l2)
    if 1e-8<t<1-1e-8 and np.linalg.norm(np.array(p)-pa-v*t)<1e-8:nodes.append((t,k))
   chain=[a]+[k for t,k in sorted(nodes)]+[b];key=tuple(sorted((a,b)))
   if nodes and key in boundary:
    boundary.remove(key);last=a
    for t,k in sorted(nodes):
     fixed[k]=fixed[a]*(1-t)+fixed[b]*t;split_kept(source_ids[last],source_ids[b],source_ids[k]);last=k
    boundary.update(tuple(sorted(e))for e in zip(chain,chain[1:]))
   noded.extend(zip(chain,chain[1:]))
  edges=noded
  tri=Delaunay(np.array(points));present={tuple(sorted((int(a),int(b))))for t in tri.simplices for a,b in zip(t,np.roll(t,-1))};missing=[e for e in edges if tuple(sorted(e))not in present]
  if not missing:break
  new=[]
  for a,b in edges:
   key=tuple(sorted((a,b)))
   if key in present:new.append((a,b));continue
   m=add((np.array(points[a])+points[b])/2);assert m not in (a,b)
   if key in boundary:
    boundary.remove(key);boundary.update([tuple(sorted((a,m))),tuple(sorted((m,b)))]);fixed[m]=(fixed[a]+fixed[b])/2;split_kept(source_ids[a],source_ids[b],source_ids[m])
   new.extend([(a,m),(m,b)])
  edges=new
 else:raise AssertionError('Projected constraint recovery failed')
 def displacement(p):
  amount=0.
  for face in proposals:
   poly=face['native_outline']
   if not inside(p,poly):continue
   distances=[]
   for a,b in zip(poly,poly[1:]+poly[:1]):
    a=np.array(a);b=np.array(b);v=b-a;t=np.clip(np.dot(p-a,v)/np.dot(v,v),0,1);distances.append(float(np.linalg.norm(p-a-t*v)))
   span=np.ptp(poly,axis=0);radius=max(2.,float(min(span))*.30);amount=max(amount,min(12.,float(min(span))*.70)*math.sin(min(1.,min(distances)/radius)*math.pi/2))
  return amount
 # Discard the old interior loft depths. A positive-weight harmonic body
 # interpolates only the retained boundary/contact vertices, then complete
 # source-traced faces supply convex depth. Old sample lines no longer create
 # artificial broad diagonal walls between narrow courses.
 valid_tri=[];adj=[set()for p in points]
 for t in tri.simplices:
  q=np.array(points)[t];u=q[1]-q[0];v=q[2]-q[0]
  if abs(u[0]*v[1]-u[1]*v[0])<1e-8 or not inside(q.mean(0),polygon):continue
  valid_tri.append(t)
  for a,b in zip(t,np.roll(t,-1)):adj[int(a)].add(int(b));adj[int(b)].add(int(a))
 unknown=[i for i in range(len(points))if i not in fixed];indices={i:j for j,i in enumerate(unknown)};matrix=lil_matrix((len(unknown),len(unknown)));rhs=np.zeros(len(unknown))
 for i in unknown:
  assert adj[i];j=indices[i]
  for neighbor in adj[i]:
   weight=1/max(1e-6,float(np.linalg.norm(np.array(points[i])-points[neighbor])));matrix[j,j]+=weight
   if neighbor in fixed:rhs[j]+=weight*fixed[neighbor][2]
   else:matrix[j,indices[neighbor]]-=weight
 solution=spsolve(matrix.tocsr(),rhs);assert np.isfinite(solution).all();assert solution.min()>=min(v[2]for v in fixed.values())-1e-6 and solution.max()<=max(v[2]for v in fixed.values())+1e-6
 for i,p in enumerate(points):
  if i in fixed:w=fixed[i]
  else:
   z=float(solution[indices[i]]);assert math.isfinite(z),(owner,p);w=np.array([p[0],-(p[1]+z*C)/S,z])+RAY*displacement(np.array(p))
  world[source_ids[i]]=w.tolist()
 front=[]
 for t in tri.simplices:
  q=np.array(points)[t];u=q[1]-q[0];v=q[2]-q[0]
  if abs(u[0]*v[1]-u[1]*v[0])<1e-8:continue
  if not inside(np.mean(np.array(points)[t],axis=0),polygon):continue
  ids=[source_ids[int(i)]for i in t];a,b,c=np.array(world)[ids];
  if np.dot(np.cross(b-a,c-a),RAY)<0:ids=ids[::-1]
  front.append(ids)
 faces=kept+front;ec=Counter(tuple(sorted((a,b)))for f in faces for a,b in zip(f,f[1:]+f[:1]));oriented=Counter((a,b)for f in faces for a,b in zip(f,f[1:]+f[:1]));assert set(ec.values())=={2},[(list(e),count,project(np.array(world)[list(e)]).tolist(),[i for i,f in enumerate(kept)if set(e)<=set(f)],[i for i,f in enumerate(front)if set(e)<=set(f)])for e,count in ec.items()if count!=2];assert all(oriented[(b,a)]==1 for a,b in oriented);assert len(world)-len(ec)+len(faces)==2
 updated=[]
 for name,ids in trace_local:
  path=[]
  for a,b in zip(ids,ids[1:]):
   pa=np.array(points[a]);v=np.array(points[b])-pa;l2=np.dot(v,v);found=[]
   for i,p in enumerate(points):
    t=np.dot(np.array(p)-pa,v)/l2
    if -1e-8<=t<=1+1e-8 and np.linalg.norm(np.array(p)-pa-v*t)<1e-7:found.append((float(t),source_ids[i]))
   segment=[j for t,j in sorted(found)];path.extend(segment if not path else segment[1:])
  updated.append({'id':name,'vertices':path})
 volume=sum(float(np.dot(world[f[0]],np.cross(world[f[1]],world[f[2]])))/6 for f in faces);assert volume>0
 out=dict(mesh);out.update(vertices=world,faces=faces,traces=updated,signed_volume=volume,topology='Closed source-plane front reconstruction joined to retained side/back/bottom; complete convex rock faces, not course lofts.',front_reconstruction={'previous_sha256':hashlib.sha256(json.dumps(mesh,sort_keys=True).encode()).hexdigest(),'new_front_triangles':len(front),'native_boundary':polygon,'retained_boundary_max_error':max(float(np.max(np.abs(np.array(world[i])-old[i])))for i in border),'face_proposals':[p['id']for p in proposals],'perimeter_splits':len(fixed)-len(border),'old_source_paths_retained':True,'interior_depth':'Harmonic positive-weight boundary interpolation plus whole boulder faces; previous loft interior depths discarded'})
 return out

def revise_inferred_boundaries(data,east,proposal):
 changes=[];south=data['54'];v=south['vertices'];n=len(v)//len(south['ring_names']);nf=n-3;ivy=next(r['native_outline']for r in proposal['faces']if r['id']=='south-ivy-boulder')
 for row in range(1,len(south['ring_names'])-1):
  index=row*n+nf-1;old=np.array(v[index]);sy=-old[1]*S-old[2]*C;crossings=[]
  for a,b in zip(ivy,ivy[1:]+ivy[:1]):
   if min(a[1],b[1])<=sy<=max(a[1],b[1]) and a[1]!=b[1]:crossings.append(a[0]+(sy-a[1])/(b[1]-a[1])*(b[0]-a[0]))
  if not crossings:continue
  x=max(old[0],max(crossings)+2.)
  if x-old[0]<1e-7:continue
  for i in [index,row*n+nf]:
   previous=list(v[i]);v[i][0]+=x-old[0];changes.append({'owner':54,'vertex':i,'old_world':previous,'new_world':list(v[i]),'reason':'Inferred shoulder continued around the full ivy-obscured rock outline; source outline uncertainty remains explicit.'})
 # Only the inferred top front lip at the far eastern end moves. This is
 # separate from the measured shared crest over x175..340 and its endpoints.
 n=len(east['vertices'])//len(east['ring_names']);nf=n-3
 for i in range(nf):
  old=np.array(east['vertices'][i]);x=old[0]
  if x<603:continue
  sy=-old[1]*S-old[2]*C;target=min(sy,float(np.interp(x,[603,610,613,616],[191,192,195,199])));z=old[2]+(sy-target)/C;new=[float(x),float(old[1]),float(z)]
  if np.max(np.abs(np.array(new)-old))>1e-7:changes.append({'owner':52,'vertex':i,'old_world':old.tolist(),'new_world':new,'reason':'Inferred top lip raised to encompass visible top-right boulder, preserving world depth and unchanged main-bank join.'});east['vertices'][i]=new
 data['inferred_boundary_revision']={'changes':changes,'authority':'Visible face proposals support correcting old inferred boundary, not measured shared crests or source2D samples.','measured_shared_crest_unchanged':True}
 return changes

def main():
 assert not O.exists()and not E.exists();O.mkdir();E.mkdir();d=json.loads((B/'bank-shoulder-lofts-cpu-v7/geometry.json').read_text());east=json.loads((B/'bank-east-loft-cpu-v8/geometry.json').read_text());proposal=json.loads((B/'bank-visible-face-traces-v1/faces.json').read_text());assert proposal['source_sha256']==hashlib.sha256((B.parent/'baseline/covered.png').read_bytes()).hexdigest();revise_inferred_boundaries(d,east,proposal);old54=d['54'];traces54=[{'id':r['id'],'source':project(np.array(old54['vertices'])[r['vertices']]).tolist()}for r in old54['traces']];d['54']=rebuild(old54,54,[r for r in proposal['faces']if r['owner']==54],traces54)
 traces52=[{'id':r['id'],'source':project(np.array(d['52']['vertices'])[r['vertices']]).tolist()}for r in d['52']['traces']if r['id'].startswith('east-')];east=rebuild(east,52,[r for r in proposal['faces']if r['owner']==52],traces52)
 # The full52 validation plan stores the actual reconstructed source path too;
 # its eastern construction faces are replaced by this separate closed mesh.
 for r in d['52']['traces']:
  if not r['id'].startswith('east-'):continue
  matched=next(t for t in east['traces']if t['id']==r['id']);path=[east['vertices'][i]for i in matched['vertices']];px=project(np.array(path))
  for index in r['vertices']:
   oldxy=project(np.array([d['52']['vertices'][index]]))[0];distance=np.linalg.norm(px-oldxy,axis=1);assert distance.min()<1e-7;d['52']['vertices'][index]=path[int(distance.argmin())]
  r['source_path_world']=path
 d['visible_face_provenance']={'source_proposals_sha256':hashlib.sha256((B/'bank-visible-face-traces-v1/faces.json').read_bytes()).hexdigest(),'depth':'Convex face depth inferred; exact native sample paths and original closed perimeter retained.','prior_status':'Old paired trace labels are not complete rock-course anatomy.'};(O/'geometry.json').write_text(json.dumps(d,indent=2)+'\n');(E/'geometry.json').write_text(json.dumps(east,indent=2)+'\n');print({'south':d['54']['front_reconstruction'],'east':east['front_reconstruction']})
if __name__=='__main__':main()
