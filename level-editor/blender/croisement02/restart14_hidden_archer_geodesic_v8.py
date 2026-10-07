"""CPU surface-edge paths and bounded clearance for a private climbing envelope."""
import json,math,hashlib,shutil
from pathlib import Path
import numpy as np
from scipy.spatial import cKDTree
from scipy.sparse import coo_matrix
from scipy.sparse.csgraph import dijkstra,connected_components
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
BASE=Path(__file__).resolve().parents[2]/'work/croisement02-refinement/restart14-hidden-archer'
SRC=BASE/'surface-v8';DEST=BASE/'geodesic-v8-cpu';SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35));RAY=np.array([0,-COS,SIN])
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def budget(n=0):
 assert shutil.disk_usage(BASE).free-n>=10*1024**3
 assert sum(p.stat().st_size for p in DEST.rglob('*') if p.is_file())+n<8*1024**2

def main():
 budget(2*1024**2);assert not DEST.exists();authority=json.loads((SRC/'extraction.json').read_text());assert authority['status']=='PASS';assert sha(SRC/'surfaces.npz')==authority['surfaces_sha256'];data=np.load(SRC/'surfaces.npz');v=data['vertices0'];t=data['triangles0']
 # Weld coincident mesh vertices for continuous topological path analysis only.
 unique,inverse=np.unique(np.round(v,5),axis=0,return_inverse=True);tri=inverse[t];valid=(unique[:,0]>=95)&(unique[:,2]>=43.9)
 edge=np.unique(np.sort(np.concatenate([tri[:,[0,1]],tri[:,[1,2]],tri[:,[2,0]]]),axis=1),axis=0);edge=edge[valid[edge].all(1)];edge=edge[edge[:,0]!=edge[:,1]];length=np.linalg.norm(unique[edge[:,0]]-unique[edge[:,1]],axis=1);graph=coo_matrix((np.tile(length,2),(edge.ravel(order='F'),edge[:,::-1].ravel(order='F'))),shape=(len(unique),len(unique))).tocsr();tree=cKDTree(unique);paths=[]
 for x in [110,125,145]:
  foot_candidates=np.where(valid&(unique[:,2]<46))[0];tip_candidates=np.where(valid&(unique[:,2]>135))[0]
  foot=int(foot_candidates[np.argmin(np.linalg.norm(unique[foot_candidates]-[x,-185,44],axis=1))]);tip=int(tip_candidates[np.argmin(np.linalg.norm(unique[tip_candidates]-[x,-250,140],axis=1))]);distance,pred=dijkstra(graph,indices=foot,return_predecessors=True);route=[];node=tip
  if np.isfinite(distance[tip]):
   while node!=foot:route.append(node);node=int(pred[node]);assert node>=0
   route.append(foot);route=route[::-1]
  points=unique[route];paths.append(dict(target_x=x,connected=bool(len(route)),surface_edge_length=float(distance[tip]) if len(route) else None,points=points.tolist(),foot_z=float(unique[foot,2]),foot_to_bank_height=abs(float(unique[foot,2])-43.9485),maximum_segment=float(np.linalg.norm(np.diff(points,axis=0),axis=1).max()) if len(route)>1 else None))
 fig,axes=plt.subplots(1,3,figsize=(14,5),layout='constrained');sample=unique[valid][::10];axes[0].scatter(sample[:,1],sample[:,2],s=.2,c='#999999')
 for p in paths:
  q=np.array(p['points'])
  if len(q):axes[0].plot(q[:,1],q[:,2],lw=1,label=str(p['target_x']))
 axes[0].legend();axes[0].set_aspect('equal');axes[0].set_title('Three measured rock-edge paths');axes[0].set_xlabel('World Y');axes[0].set_ylabel('World Z')
 states=[]
 for ax,state in zip(axes[1:],['initial','applied']):
  path=BASE/f'depth-v8-cpu-compact/{state}-constraints.json';r=json.loads(path.read_text());front=np.array(r['front']);support=np.array(r['support']);raygap=(front-support)@RAY;nearest=tree.query(front)[0];xlower=np.maximum(0,front[:,0]-v[:,0].max());bankgap=front[:,2]-43.9492
  # A point ahead of the independently alpha-tested first hit cannot be behind
  # another opaque hit on that same ray. This proves centers, not leaf volumes.
  assert raygap.min()>1.49
  ax.scatter(front[:,0],front[:,2],c=np.minimum(nearest,50),s=2,cmap='magma',vmin=0,vmax=50);ax.axvline(v[:,0].max(),c='blue',lw=.7);ax.set_aspect('equal');ax.set_title(state+': vertex-distance upper bound');ax.set_xlabel('World X');ax.set_ylabel('World Z')
  states.append(dict(state=state,constraints_sha256=sha(path),native_centers=len(front),center_ray_clearance_min=float(raygap.min()),center_ray_clearance_max=float(raygap.max()),within_four_units_of_rock_vertex=int((nearest<=4).sum()),outside_rock_x_by_over_four=int((xlower>4).sum()),outside_rock_x_by_over_four_and_over_four_above_bank=int(((xlower>4)&(bankgap>4)).sum()),maximum_x_distance_lower_bound=float(xlower.max()),rock_vertex_distance_max_upper_bound=float(nearest.max())))
 DEST.mkdir();budget(1048576);fig.savefig(DEST/'surface-paths-and-support.png',dpi=100);plt.close(fig)
 report=dict(status='HOLD; measured surface paths exist but compact leaf support is not certified',extraction_sha256=sha(SRC/'extraction.json'),surface_sha256=authority['surfaces_sha256'],paths=paths,states=states,image_sha256=sha(DEST/'surface-paths-and-support.png'),conclusions=['All7073 proposed opaque centers lie ahead of their alpha-aware measured receiver first hits. This is not a volumetric collision certificate.','Surface-edge paths are on exact evaluated rock triangles and are upper bounds on surface geodesic distance. A tube offset can still penetrate the surface and must be tested before modeling.','Many right-side leaf centers extend over4 units beyond the rock X bounds and over4 above the bank; the current compact envelope cannot be claimed fully supported by either receiver.','Do not span remaining gaps with parallel strands. Fit an explicit few-stem cantilever or revise the common envelope with continuous volumetric clearance before building.','No models, renders, original receiver edits, or canonical changes were performed.'])
 budget(1048576);(DEST/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(dict(paths=[{k:v for k,v in p.items() if k!='points'} for p in paths],states=states),indent=2))
if __name__=='__main__':main()
