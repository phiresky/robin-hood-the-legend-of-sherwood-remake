"""Replace inferred mid-stem ring ripples with a smoothly blended round section."""
import hashlib
import json
from pathlib import Path
import numpy as np
from restart2_tree08_local_fork import audit

R=Path(__file__).resolve().parents[3]/'level-editor/work/croisement01-refinement/restart2'
source=R/'tree08-connected-tip-cleanup-v2/candidate.npz'
out=R/'tree08-main-stem-radial-fairing-v3-crossing-buffer'
assert not out.exists()
data=np.load(source);v,f=data['vertices'],data['faces']
def smooth(x):
    x=np.clip(x,0,1)
    return x*x*(3-2*x)
s,c=np.sin(np.radians(35)),np.cos(np.radians(35))
native_y=v@np.array([0,-s,-c])
z=v[:,2]
center=np.column_stack((562+2*np.exp(-((z-120)/30)**2),np.full(len(v),-370/s)))
radial=v[:,:2]-center
radius=np.linalg.norm(radial,axis=1)
target=24.5-.012*(z-100)
weight=.95*smooth((z-80)/30)*smooth((185-z)/30)
weight*=smooth((radius-5)/10)*smooth((42-radius)/14)
weight*=smooth((315-native_y)/15)
replacement=radius+weight*(target-radius)
vertices=v.copy()
vertices[:,:2]=center+radial*(replacement/np.maximum(radius,1e-12))[:,None]
origin=np.array([552.,-672.,235.])
vertices=(vertices-origin).astype(np.float32).astype(float)+origin
vertices[weight==0]=v[weight==0]
assert np.array_equal(vertices[native_y>=315],v[native_y>=315])
proposed=vertices-v
original=v[f]
original_normal=np.cross(original[:,1]-original[:,0],original[:,2]-original[:,0])
area2=np.einsum('ij,ij->i',original_normal,original_normal)
damping=np.ones(len(v))
crossing=json.loads((R/'tree08-main-stem-radial-fairing-v2-oriented/intersection-guard.json').read_text())
pin=np.zeros(len(v),bool)
pin[np.unique(f[np.array([row['faces'] for row in crossing['new_intersections']])])]=True
for _ in range(2):pin[np.unique(f[pin[f].any(axis=1)])]=True
damping[pin]=0
halo=pin.copy()
for value in [.25,.5,.75]:
    expanded=halo.copy();expanded[np.unique(f[halo[f].any(axis=1)])]=True
    damping[expanded&~halo]=value
    halo=expanded
for iteration in range(80):
    vertices=(v+proposed*damping[:,None]-origin).astype(np.float32).astype(float)+origin
    vertices[damping<1e-3]=v[damping<1e-3]
    vertices[weight==0]=v[weight==0]
    triangle=vertices[f]
    normal=np.cross(triangle[:,1]-triangle[:,0],triangle[:,2]-triangle[:,0])
    ratio=np.einsum('ij,ij->i',normal,original_normal)/area2
    folded=ratio<.05
    if not folded.any():break
    damping[np.unique(f[folded])]*=.5
assert not folded.any()
check=audit(vertices,f)
assert not any(check[k] for k in ['nonmanifold_edges','winding_errors','zero_area'])
out.mkdir();np.savez_compressed(out/'candidate.npz',vertices=vertices,faces=f,before_vertices=v)
report=dict(status='PRIVATE_TRUNK_ANATOMY_TRIAL_PENDING_COVERAGE_AND_INTERSECTIONS',parent_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),candidate_sha256=hashlib.sha256((out/'candidate.npz').read_bytes()).hexdigest(),topology=check,orientation_damping_iterations=iteration,minimum_oriented_area_ratio=float(ratio.min()),proposed_field_vertices=int((weight>0).sum()),changed_vertices=int(np.any(vertices!=v,axis=1).sum()),maximum_displacement=float(np.linalg.norm(vertices-v,axis=1).max()),lower_root_positions_exact=True,scope='Only inferred mid-stem cross section near the continuous source stem. Smooth radial field blends into retained branches and exact lower bank-contact geometry. Native artwork must be reprojected and every owned source pixel independently checked.')
(out/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2),flush=True)
