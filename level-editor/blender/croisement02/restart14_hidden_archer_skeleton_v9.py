"""Plan a continuous depth envelope and sparse rooted climbing hierarchy on CPU."""
import json, math, hashlib, shutil
from pathlib import Path
import numpy as np
from PIL import Image
from scipy.sparse import coo_matrix
from scipy.sparse.csgraph import dijkstra, connected_components
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
BASE=Path(__file__).resolve().parents[2]/'work/croisement02-refinement/restart14-hidden-archer'
DEST=BASE/'skeleton-v9-cpu';SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35));RAY=np.array([0,-COS,SIN])
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def budget(extra=0):
 assert shutil.disk_usage(BASE).free-extra>=8*1024**3
 assert sum(p.stat().st_size for p in DEST.rglob('*') if p.is_file())+extra<2*1024**2

def main():
 budget(1*1024**2);assert not DEST.exists()
 authority=BASE/'audit-v1/source-authority.json';source=next(p for p in json.loads(authority.read_text())['profiles'] if p['profile'].endswith('05'))
 measured=BASE/'audit-v1/substrate-first-hit-v1/report.json';samples=next(p['samples'] for p in json.loads(measured.read_text())['profiles'] if p['profile'].endswith('05'));lookup={tuple(p['pixel']):p for p in samples}
 x0,y0,x1,y1=98,-24,204,104;yy,xx=np.mgrid[y0:y1,x0:x1];field=np.maximum(44.9,145-1.18*(yy+.5))
 for s in samples:
  x,y=s['pixel'];field[y-y0,x-x0]=max(field[y-y0,x-x0],s['world'][2]+3*SIN)
 # Least 1.35-Lipschitz majorant; unlike a moving maximum this has no cliff
 # discontinuity and never lowers measured native clearance requirements.
 for iteration in range(250):
  old=field.copy()
  for dy,dx in [(1,0),(-1,0),(0,1),(0,-1),(1,1),(-1,-1),(1,-1),(-1,1)]:
   rolled=np.roll(old,(dy,dx),(0,1))-1.35*math.hypot(dx,dy)
   if dy>0:rolled[:dy,:]=-1e6
   if dy<0:rolled[dy:,:]=-1e6
   if dx>0:rolled[:,:dx]=-1e6
   if dx<0:rolled[:,dx:]=-1e6
   field=np.maximum(field,rolled)
  if np.max(field-old)<1e-7:break
 DEST.mkdir();plans=[];fig,axes=plt.subplots(2,2,figsize=(11,10),layout='constrained')
 for row,st in enumerate(source['states']):
  rgba=np.array(Image.open(st['sprite_source']));assert sha(Path(st['sprite_source']))==st['sprite_sha256'];oy,ox=st['native_top_left'][1],st['native_top_left'][0];py,px=np.where(rgba[:,:,3]>=128);pix=np.column_stack([px+ox,py+oy]);height=field[pix[:,1]-y0,pix[:,0]-x0];front=np.column_stack([pix[:,0]+.5,-((pix[:,1]+.5)+height*COS)/SIN,height]);support=np.array([lookup[tuple(p)]['world'] for p in pix]);index={tuple(p):i for i,p in enumerate(pix)};edges=[]
  for (x,y),i in index.items():
   for dx,dy in [(1,0),(0,1),(1,1),(-1,1)]:
    j=index.get((x+dx,y+dy))
    if j is not None:edges.append((i,j))
  edges=np.array(edges);length=np.linalg.norm(front[edges[:,0]]-front[edges[:,1]],axis=1);graph=coo_matrix((np.tile(length,2),(edges.ravel(order='F'),edges[:,::-1].ravel(order='F'))),shape=(len(front),len(front))).tocsr();_,labels=connected_components(graph);mainlabel=np.argmax(np.bincount(labels));domain=np.where(labels==mainlabel)[0];root=int(domain[np.argmin(front[domain,2])]);distance,pred=dijkstra(graph,indices=root,return_predecessors=True)
  # Three primary routes target measured rock path tips and side-shrub crown;
  # only six additional terminal routes, never one branch per source pixel.
  target_pixels=[[110,17],[125,12],[170,5],[113,42],[135,38],[173,35],[135,64],[166,50],[185,23]]
  route_edges=set();targets=[]
  for q in target_pixels:
   target=int(domain[np.argmin(np.linalg.norm(pix[domain]-q,axis=1))]);targets.append(target);i=target
   while i!=root:
    parent=int(pred[i]);assert parent>=0;route_edges.add(tuple(sorted((i,parent))));i=parent
  # Preserve degree junctions; retain every eighth path vertex as a bend.
  adjacency={}
  for a,b in route_edges:adjacency.setdefault(a,[]).append(b);adjacency.setdefault(b,[]).append(a)
  junctions={n for n,v in adjacency.items() if len(v)!=2}|{root}|set(targets);segments=[];seen=set()
  for start in junctions:
   for neighbor in adjacency[start]:
    if tuple(sorted((start,neighbor))) in seen:continue
    chain=[start,neighbor];seen.add(tuple(sorted((start,neighbor))));last=start;node=neighbor
    while node not in junctions:
     nxt=next(n for n in adjacency[node] if n!=last);seen.add(tuple(sorted((node,nxt))));chain.append(nxt);last,node=node,nxt
    sparse=chain[::4]
    if sparse[-1]!=chain[-1]:sparse.append(chain[-1])
    segments.append(sparse)
  bank_root=front[root].copy();bank_root[2]=43.8
  ax=axes[row,0];ax.imshow(rgba,extent=[ox,ox+rgba.shape[1],oy+rgba.shape[0],oy]);
  for chain in segments:
   q=pix[chain]+.5;ax.plot(q[:,0],q[:,1],lw=.8,c='cyan')
  ax.set_title(st['state']+': sparse rooted hierarchy over source');ax=axes[row,1];ax.scatter(front[:,1],front[:,2],s=.5,c='#528542')
  for chain in segments:
   q=front[chain]-RAY*2;ax.plot(q[:,1],q[:,2],lw=.7,c='#774421')
  ax.set_aspect('equal');ax.set_title('Continuous envelope and few branch routes');ax.set_xlabel('World Y');ax.set_ylabel('World Z')
  plan=dict(state=st['state'],source=st['sprite_source'],source_sha256=st['sprite_sha256'],source_top_left=st['native_top_left'],pixels=pix.tolist(),front=np.round(front,6).tolist(),segments=segments,root_index=root,bank_root=bank_root.tolist(),native_centers=len(front),main_component_centers=len(domain),tiny_components_excluded_from_branch_routes=int(len(front)-len(domain)),branch_routes=len(segments),maximum_adjacent_center_distance=float(length.max()),depth_extent=float(np.ptp(front[:,1])),width=float(np.ptp(front[:,0])),height=float(np.ptp(front[:,2])),receiver_ray_clearance_min=float(((front-support)@RAY).min()),geometry_status='PLAN ONLY; exact closed support/leaf volume clearance not tested')
  budget(512*1024);(DEST/f'{st["state"]}-plan.json').write_text(json.dumps(plan,separators=(',',':'))+'\n');plans.append({k:v for k,v in plan.items() if k not in ['pixels','front','segments']})
 budget(512*1024);np.savez_compressed(DEST/'envelope.npz',height=field,origin=[x0,y0]);fig.savefig(DEST/'skeleton-and-source.png',dpi=100);plt.close(fig)
 (DEST/'report.json').write_text(json.dumps(dict(status='CPU PLAN; pending surface clearance',plans=plans,authority_sha256=sha(authority),measured_sha256=sha(measured),surface_extraction_sha256=sha(BASE/'surface-v8/extraction.json'),geodesic_sha256=sha(BASE/'geodesic-v8-cpu/report.json'),image_sha256=sha(DEST/'skeleton-and-source.png')),indent=2)+'\n');print(json.dumps(plans,indent=2))
if __name__=='__main__':main()
