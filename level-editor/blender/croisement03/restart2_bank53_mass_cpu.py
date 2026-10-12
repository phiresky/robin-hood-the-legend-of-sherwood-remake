"""Closed bank53 whole-mass CPU proposal with exact retained 2D source marks."""
import argparse,hashlib,json,math,sys
from pathlib import Path
from collections import Counter
import numpy as np
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/active/croisement03';S=math.sin(math.radians(35));C=math.cos(math.radians(35))
sys.path.insert(0,str(Path(__file__).parent))
from restart2_bank_continuous_strata import triangulate

def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def cross(a,b):return a[0]*b[1]-a[1]*b[0]
def main():
 p=argparse.ArgumentParser();p.add_argument('--output',type=Path,required=True);a=p.parse_args();assert not a.output.exists()
 oldpath=B/'restart2/bank-continuous-strata-plan-v1/geometry.json';proposalpath=B/'restart2/bank53-source-masses-v1/receipt.json';levelpath=B/'baseline/Croisement03.rhp.json';old=json.loads(oldpath.read_text());proposal=json.loads(proposalpath.read_text());level=json.loads(levelpath.read_text());ground=level['sight_obstacles'][53]['points']
 left=[ground[4]['x'],ground[4]['y']];right=[ground[3]['x'],ground[3]['y']]
 # The outer front includes full broad masses. Between exposed outlines its edge is inferred.
 boundary=[left,[128,394],[124,379],[126,360],[131,348],[146,338],[128,324],[124,314],[123,299],[132,292],[145,288],[151,278],[151,267],[158,260],[169,257],[181,261],[193,268],[202,276],[207,282],[205,291],[207,305],[209,319],[207,330],[193,345],[201,359],[203,374],[208,390],right]
 xy=[list(q) for q in boundary];faces=triangulate(xy)
 def insert(q):
  q=np.array(q,dtype=float)
  for i,v in enumerate(xy):
   if np.linalg.norm(q-v)<1e-7:return i
  for j,f in enumerate(faces):
   t=np.array([xy[i] for i in f]);v=t[1]-t[0];w=t[2]-t[0];d=cross(v,w)
   if abs(d)<1e-10:continue
   u=cross(q-t[0],w)/d;z=cross(v,q-t[0])/d
   if min(u,z,1-u-z)<-1e-8:continue
   near=[k for k,val in enumerate([1-u-z,u,z]) if abs(val)<1e-8]
   index=len(xy);xy.append(q.tolist())
   if near:
    k=near[0];edge=tuple(sorted([f[(k+1)%3],f[(k+2)%3]]));adj=[n for n,h in enumerate(faces) if all(v in h for v in edge)]
    for n in reversed(adj):
     h=faces.pop(n);other=next(v for v in h if v not in edge);faces.extend([[edge[0],index,other],[index,edge[1],other]])
   else:
    faces.pop(j);faces.extend([[f[0],f[1],index],[f[1],f[2],index],[f[2],f[0],index]])
   return index
  return None
 trace_specs=[]
 for mark in proposal['retained_2d_band_marks']:
  for role in ['rim','lower']:
   ids=[insert(q) for q in mark[role]];assert all(i is not None for i in ids),(mark['id'],role,[(q,i) for q,i in zip(mark[role],ids) if i is None])
   trace_specs.append({'id':mark['id']+'-'+role,'vertices':ids,'source_xy':mark[role]})
 for mass in proposal['masses']:
  for q in mass['outline']:insert(q)
 for y in range(259,409,4):
  for x in range(124,209,4):insert([x,y])
 # Improve interior triangulation by constrained Delaunay flips; boundary edges never move.
 pts=np.array(xy);flip_total=0;locked=set()
 for sweep in range(120):
  adjacency={}
  for j,f in enumerate(faces):
   for u,v in zip(f,f[1:]+f[:1]):adjacency.setdefault(tuple(sorted((u,v))),[]).append(j)
  touched=set();flipped=0
  for (u,v),pair in adjacency.items():
   if len(pair)!=2 or (u,v) in locked or any(j in touched for j in pair):continue
   j,k=pair;c0=next(i for i in faces[j] if i not in (u,v));d0=next(i for i in faces[k] if i not in (u,v));aa,bb,cc,dd=pts[[u,v,c0,d0]]
   if cross(bb-aa,cc-aa)*cross(bb-aa,dd-aa)>=-1e-10 or cross(dd-cc,aa-cc)*cross(dd-cc,bb-cc)>=-1e-10:continue
   q=np.array([aa-dd,bb-dd,cc-dd]);inc=float(np.linalg.det(np.column_stack((q,(q*q).sum(1)))))
   if inc*cross(bb-aa,cc-aa)<=1e-7:continue
   faces[j]=[c0,d0,u];faces[k]=[d0,c0,v];touched.update(pair);flipped+=1
  flip_total+=flipped
  if not flipped:break
 else:raise RuntimeError('Constrained edge flips did not converge')
 # Recover every retained band segment as actual mesh edges before quality flips.
 def crosses(i,j,u,v):
  aa,bb,cc,dd=pts[[i,j,u,v]]
  return cross(bb-aa,cc-aa)*cross(bb-aa,dd-aa)<-1e-10 and cross(dd-cc,aa-cc)*cross(dd-cc,bb-cc)<-1e-10
 for trace in trace_specs:
  original=trace['vertices'][:];expanded=[]
  for u,v in zip(original,original[1:]):
   delta=pts[v]-pts[u];candidates=[]
   for i,q in enumerate(pts):
    t=float(np.dot(q-pts[u],delta)/np.dot(delta,delta))
    if -1e-8<=t<=1+1e-8 and abs(cross(delta,q-pts[u]))<1e-7:candidates.append((t,i))
   line=[i for t,i in sorted(candidates)];expanded.extend(line[:-1])
   for i,j in zip(line,line[1:]):
    target=tuple(sorted((i,j)))
    for attempt in range(120):
     adj={}
     for fi,f in enumerate(faces):
      for x,y in zip(f,f[1:]+f[:1]):adj.setdefault(tuple(sorted((x,y))),[]).append(fi)
     if target in adj:locked.add(target);break
     options=[]
     for (x,y),pair in adj.items():
      if len(pair)!=2 or (x,y) in locked or not crosses(i,j,x,y):continue
      fa,fb=pair;ca=next(k for k in faces[fa] if k not in (x,y));cb=next(k for k in faces[fb] if k not in (x,y))
      if not crosses(x,y,ca,cb):continue
      options.append((crosses(i,j,ca,cb),fa,fb,ca,cb,x,y))
     assert options,('Cannot recover band edge',i,j)
     _,fa,fb,ca,cb,x,y=sorted(options)[0];faces[fa]=[ca,cb,x];faces[fb]=[cb,ca,y]
    else:raise RuntimeError(('Band edge recovery did not converge',i,j))
  expanded.append(original[-1]);trace['original_mark_vertices']=original;trace['original_source_xy']=trace['source_xy'];trace['vertices']=expanded;trace['source_xy']=[xy[i] for i in expanded]

 def height(x,y):
  bottom=left[1]+(right[1]-left[1])*(x-left[0])/(right[0]-left[0]);v=max(0.,min(1.,(bottom-y)/(bottom-257.)))
  h=58*v
  for cx,cy,rx,ry,amp in [(177,277,26,19,5),(165,309,42,23,7),(169,339,22,10,-3),(164,364,38,20,7),(177,390,31,15,4)]:h+=amp*math.exp(-2*((x-cx)**2/rx**2+(y-cy)**2/ry**2))*min(1,v*8)
  return max(0.,min(62.,h))
 vertices=[]
 for x,y in xy:
  h=0 if [x,y] in [left,right] else height(x,y);vertices.append([x,-(y+h)/S,h/C])
 front_count=len(vertices)
 # Winding of the front is made consistent in source XY before global orientation.
 for f in faces:
  if cross(np.array(xy[f[1]])-xy[f[0]],np.array(xy[f[2]])-xy[f[0]])<0:f.reverse()
 edges=Counter(tuple(sorted((x,y))) for f in faces for x,y in zip(f,f[1:]+f[:1]));neighbors={}
 for (x,y),count in edges.items():
  if count==1:neighbors.setdefault(x,[]).append(y);neighbors.setdefault(y,[]).append(x)
 endpoint=len(boundary)-1;arc=[0];previous=endpoint
 while arc[-1]!=endpoint:
  nxt=next(i for i in neighbors[arc[-1]] if i!=previous);previous=arc[-1];arc.append(nxt)
 rear=[0]
 for i in arc[1:-1]:
  v=np.array(vertices[i]);amount=35*min(1.,v[2]/25.);v[1]+=amount;rear.append(len(vertices));vertices.append(v.tolist())
 rear.append(endpoint)
 for i in range(len(arc)-1):
  quad=[arc[i],rear[i],rear[i+1],arc[i+1]];clean=[]
  for v in quad:
   if v not in clean:clean.append(v)
  if len(clean)==3:faces.append(clean)
  else:faces.extend([[clean[0],clean[1],clean[2]],[clean[0],clean[2],clean[3]]])
 ground_ids={4:0,3:len(boundary)-1}
 for i in [2,1,0,6,5]:ground_ids[i]=len(vertices);q=ground[i];vertices.append([q['x'],-q['y']/S,0.])
 backloop=rear+[ground_ids[i] for i in [2,1,0,6,5]]
 center=len(vertices);vertices.append([166.,-350/S,27.])
 for x,y in zip(backloop,backloop[1:]+backloop[:1]):faces.append([center,x,y])
 footprint=[ground_ids[i] for i in range(7)];bottomfaces=triangulate([vertices[i] for i in footprint]);faces.extend([[footprint[i] for i in f] for f in bottomfaces])
 # Orient the whole connected shell by edge propagation.
 links={}
 for i,f in enumerate(faces):
  for x,y in zip(f,f[1:]+f[:1]):links.setdefault(tuple(sorted((x,y))),[]).append((i,x,y))
 assert set(map(len,links.values()))=={2}
 flips={0:False};stack=[0]
 while stack:
  i=stack.pop();f=faces[i]
  for x,y in zip(f,f[1:]+f[:1]):
   for j,u,v in links[tuple(sorted((x,y)))]:
    if j==i:continue
    wanted=flips[i] ^ (u==x)
    if j in flips:assert flips[j]==wanted
    else:flips[j]=wanted;stack.append(j)
 assert len(flips)==len(faces)
 faces=[f[::-1] if flips[i] else f for i,f in enumerate(faces)];vv=np.array(vertices);volume=sum(np.dot(vv[x],np.cross(vv[y],vv[z]))/6 for x,y,z in faces)
 if volume<0:faces=[f[::-1] for f in faces];volume=-volume
 areas=[np.linalg.norm(np.cross(vv[y]-vv[x],vv[z]-vv[x]))/2 for x,y,z in faces];assert min(areas)>1e-8
 errors=[]
 for trace in trace_specs:
  for i,q in zip(trace['vertices'],trace['source_xy']):v=vertices[i];errors.append(max(abs(v[0]-q[0]),abs(-v[1]*S-v[2]*C-q[1])))
 assert max(errors)<1e-9
 for i in range(7):assert np.max(np.abs(np.array(vertices[ground_ids[i]])-[ground[i]['x'],-ground[i]['y']/S,0]))<1e-9
 a.output.mkdir();report={'vertices':vertices,'faces':faces,'traces':trace_specs,'status':'HOLD_CPU_CLOSED_WHOLE_MASSES_COMBINED_CONTACT_RAYS_PENDING','source_obstacle_context':53,'front_vertex_count':front_count,'constrained_delaunay_flips':flip_total,'retained_band_mesh_edges':len(locked),'signed_volume':float(volume),'minimum_triangle_area':float(min(areas)),'euler':len(vertices)-len(links)+len(faces),'connected_components':1,'manifold_edges':True,'source_trace_max_error':max(errors),'authored_height_cap':62,'ground_footprint_vertex_ids':ground_ids,'source_ownership_assigned':False,'input_sha256':{str(p):sha(p) for p in [oldpath,proposalpath,levelpath]},'old_world_provenance':old,'construction':['Whole source front triangulation with exact old2D marks and outline/grid samples.','Smooth convex A/B/D/E authored-height lobes plus recessed C; no oldloft height scaling.','Rear shoulder offset at most35world, connected rearbulk and exact seven-point originalground footprint.'],'limits':['All frontdepths and rearclosure are inferred, not measured.','Outer source silhouette between proposed visible mass outlines remains approximate.','CPU manifold/positivevolume does not certify triangle selfintersection or cap52/ramp54 contacts.','Combined protected3446 firsthits must use unchanged oldbank baseline; do not rebaseline failures.','Old2D bandmarks preserved exactly; their old3D embedding is retained only as provenance.']};(a.output/'geometry.json').write_text(json.dumps(report,indent=2)+'\n');print({k:report[k] for k in ['status','front_vertex_count','signed_volume','minimum_triangle_area','euler','source_trace_max_error']})
if __name__=='__main__':main()
