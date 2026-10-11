"""Smooth an unowned join at a world-space scale, independent of triangle density."""
import hashlib,json
from pathlib import Path
import numpy as np
from scipy.spatial import cKDTree
from PIL import Image
from restart2_tree08_local_fork import audit
R=Path(__file__).resolve().parents[3]/'level-editor/work/croisement01-refinement/restart2'
source=R/'tree08-implicit-collar-v13-smooth-clean/candidate.npz';out=R/'tree08-implicit-collar-v16-surface-seam';assert not out.exists()
a=np.load(source);v,f=a['vertices'],a['faces'];s,c=np.sin(np.radians(35)),np.cos(np.radians(35));native=-v[:,1]*s-v[:,2]*c
core=np.asarray(Image.open(R/'tree08-semantic-source-v1/bark-core-proposal.png'))>0;y,x=np.where(core)
assert not np.any((y+11>=290)&(y+11<=350)&(x+331>=520)&(x+331<=610))
def smooth(t):
 t=np.clip(t,0,1);return t*t*(3-2*t)
weight=smooth((native-290)/15)*smooth((350-native)/15)*smooth((v[:,0]-520)/15)*smooth((610-v[:,0])/15)
vertices=v.copy();selected=np.flatnonzero(weight)
for iteration in range(12):
 cross=np.cross(vertices[f[:,1]]-vertices[f[:,0]],vertices[f[:,2]]-vertices[f[:,0]])
 normals=np.zeros_like(vertices);areas=np.zeros(len(vertices))
 for k in range(3):np.add.at(normals,f[:,k],cross);np.add.at(areas,f[:,k],np.linalg.norm(cross,axis=1))
 normals/=np.maximum(np.linalg.norm(normals,axis=1),1e-30)[:,None]
 tree=cKDTree(vertices);changed=vertices.copy()
 for i in selected:
  ids=np.array(tree.query_ball_point(vertices[i],5.));delta=vertices[ids]-vertices[i]
  w=np.exp(-np.sum(delta*delta,axis=1)/8)*areas[ids]*(normals[ids]@normals[i]>.7)
  offset=np.sum(w*(delta@normals[i]))/w.sum()
  changed[i]+=normals[i]*np.clip(offset,-.5,.5)*weight[i]*.7
 vertices=changed
 print('ITERATION',iteration,flush=True)
assert np.array_equal(vertices[native>=350],v[native>=350]);check=audit(vertices,f);out.mkdir();np.savez_compressed(out/'candidate.npz',vertices=vertices,faces=f)
report=dict(status='PRIVATE_SURFACE_SCALE_FAIRING_RECHECK_REQUIRED',source_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),candidate_sha256=hashlib.sha256((out/'candidate.npz').read_bytes()).hexdigest(),iterations=12,radius=5.,maximum_displacement=float(np.linalg.norm(vertices-v,axis=1).max()),retained_native_y_350_onward_exact=True,owned_source_pixels_in_seam_region=0,topology=check)
(out/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2),flush=True)
