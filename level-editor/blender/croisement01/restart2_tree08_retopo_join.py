"""Replace an unowned narrow trunk seam with a connected cross-section loft."""
import hashlib,json,shutil
from pathlib import Path
from collections import Counter,defaultdict
import numpy as np
from PIL import Image
from restart2_tree08_local_fork import audit
R=Path(__file__).resolve().parents[3]/'level-editor/work/croisement01-refinement/restart2';source=R/'tree08-implicit-collar-v13-smooth-clean/candidate.npz';out=R/'tree08-implicit-collar-v19-loft-seam';assert not out.exists()
assert shutil.disk_usage(R).free>=10*1024**3
assert next(int(line.split()[1]) for line in Path('/proc/meminfo').read_text().splitlines() if line.startswith('MemAvailable:'))>=6*1024**2
a=np.load(source);v,f=a['vertices'],a['faces'];s,c=np.sin(np.radians(35)),np.cos(np.radians(35));down=np.array([0.,-s,-c]);ray=np.array([0.,-c,s]);ny=v@down
core=np.asarray(Image.open(R/'tree08-semantic-source-v1/bark-core-proposal.png'))>0;y,x=np.where(core);assert not np.any((y+11>=310)&(y+11<=350)&(x+331>=520)&(x+331<=610))
vertices=v.tolist();faces=[];cache={};removed=[]
def clip(ids,cut,upper):
 result=[]
 for k,i in enumerate(ids):
  j=ids[(k+1)%len(ids)];inside=(ny[i]<=cut) if upper else (ny[i]>=cut);other=(ny[j]<=cut) if upper else (ny[j]>=cut)
  if inside:result.append(int(i))
  if inside!=other:
   key=(min(int(i),int(j)),max(int(i),int(j)),cut)
   if key not in cache:
    point=v[i]+(v[j]-v[i])*((cut-ny[i])/(ny[j]-ny[i]));cache[key]=len(vertices);vertices.append(point.tolist())
   result.append(cache[key])
 return result
for index,tri in enumerate(f):
 yy=ny[tri];xx=v[tri,0]
 if yy.max()<=310 or yy.min()>=350 or xx.max()<520 or xx.min()>610:faces.append(tri.tolist());continue
 assert xx.min()>520 and xx.max()<610,('crosses lateral limit',index)
 removed.append(index)
 for cut,upper in [(310,True),(350,False)]:
  p=clip(tri,cut,upper)
  for j in range(1,len(p)-1):faces.append([p[0],p[j],p[j+1]])
counts=Counter(tuple(sorted((t[i],t[(i+1)%3]))) for t in faces for i in range(3));edges=[e for e,n in counts.items() if n==1];adj=defaultdict(list)
for i,j in edges:adj[i].append(j);adj[j].append(i)
assert all(len(q)==2 for q in adj.values());loops=[];unseen=set(adj)
while unseen:
 start=min(unseen);loop=[start];previous=None;current=start
 while True:
  nxt=next(q for q in adj[current] if q!=previous)
  if nxt==start:break
  loop.append(nxt);previous,current=current,nxt
 unseen.difference_update(loop);loops.append(loop)
assert len(loops)==2,[len(q) for q in loops]
vv=np.asarray(vertices);loops.sort(key=lambda q:float(np.mean(vv[q]@down)));print('LOOPS',[(len(q),np.mean(vv[q]@down),vv[q].mean(0).tolist()) for q in loops],flush=True)
def ordered(loop):
 p=vv[loop];xy=np.column_stack([p[:,0],p@ray]);center=xy.mean(0);angle=np.arctan2(xy[:,1]-center[1],xy[:,0]-center[0])%(2*np.pi);order=np.argsort(angle);ids=np.asarray(loop)[order];angles=angle[order]
 assert all(tuple(sorted((int(ids[i]),int(ids[(i+1)%len(ids)])))) in counts and counts[tuple(sorted((int(ids[i]),int(ids[(i+1)%len(ids)]))))]==1 for i in range(len(ids)))
 return ids.tolist(),angles,xy[order]
upper,ua,ux=ordered(loops[0]);lower,la,lx=ordered(loops[1]);angles=np.arange(64)*2*np.pi/64

def sample(aa,xy):
 return np.column_stack([np.interp(angles,np.r_[aa[-1]-2*np.pi,aa,aa[0]+2*np.pi],np.r_[xy[-1,k],xy[:,k],xy[0,k]]) for k in range(2)])
u=sample(ua,ux);l=sample(la,lx)
rings=[(upper,ua)]
for yy in range(315,350,5):
 t=(yy-310)/40;xy=u*(1-t)+l*t;ids=[]
 for px,depth in xy:
  ids.append(len(vertices));vertices.append((np.array([px,0,0])+down*yy+ray*depth).tolist())
 rings.append((ids,angles))
rings.append((lower,la))
# Stitch cycles by angular progress; use the same monotone ordering on both ends.
for (aa,at),(bb,bt) in zip(rings,rings[1:]):
 i=j=0
 while i<len(aa) or j<len(bb):
  an=at[(i+1)%len(aa)]+(2*np.pi if i+1>=len(aa) else 0) if i<len(aa) else float('inf')
  bn=bt[(j+1)%len(bb)]+(2*np.pi if j+1>=len(bb) else 0) if j<len(bb) else float('inf')
  if an<bn:faces.append([aa[i%len(aa)],aa[(i+1)%len(aa)],bb[j%len(bb)]]);i+=1
  else:faces.append([aa[i%len(aa)],bb[(j+1)%len(bb)],bb[j%len(bb)]]);j+=1
vertices=np.asarray(vertices);faces=np.asarray(faces,dtype=int)
# Orient new triangles consistently with the retained boundary edges.
old_count=len(faces)-sum(len(a[0])+len(b[0]) for a,b in zip(rings,rings[1:]))
old_edges={(int(t[i]),int(t[(i+1)%3])) for t in faces[:old_count] for i in range(3)}
new=faces[old_count:];same=sum((int(t[i]),int(t[(i+1)%3])) in old_edges for t in new for i in range(3));opposite=sum((int(t[(i+1)%3]),int(t[i])) in old_edges for t in new for i in range(3))
if same>opposite:faces[old_count:]=new[:,::-1]
used,remap=np.unique(faces,return_inverse=True);vertices=vertices[used];faces=remap.reshape(-1,3);origin=np.array([552.,-672.,235.]);vertices=(vertices-origin).astype(np.float32).astype(float)+origin
# Preserve the tiny exterior coverage margin needed by one saved float ray.
distance=np.linalg.norm(vertices-np.array([732.5,-672.,286.8128900683257]),axis=1);w=np.clip(1-distance/4,0,1);w=w*w*(3-2*w);vertices[:,0]+=.02*w;vertices=(vertices-origin).astype(np.float32).astype(float)+origin
check=audit(vertices,faces);out.mkdir();np.savez_compressed(out/'candidate.npz',vertices=vertices,faces=faces)
report=dict(status='PRIVATE_LOFT_SEAM_REQUIRES_ALL_PROOFS',source_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),candidate_sha256=hashlib.sha256((out/'candidate.npz').read_bytes()).hexdigest(),removed_faces=len(removed),boundary_lengths=[len(q) for q in loops],native_interval=[310,350],owned_source_pixels_in_region=0,topology=check)
(out/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2),flush=True)
