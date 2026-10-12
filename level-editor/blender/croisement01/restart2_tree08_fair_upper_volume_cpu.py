"""Private upper wood fairing allowing inferred source-facing depth to change."""
import hashlib,json,shutil,sys
from pathlib import Path
import numpy as np
from scipy.sparse import coo_matrix,diags
from PIL import Image
from restart2_tree08_local_fork import audit
from restart2_tree08_junction_proof import native_depth
R=Path(__file__).resolve().parents[3]/'level-editor/work/active/croisement01/restart2'
mode=sys.argv[1] if len(sys.argv)>1 else 'taubin'
assert mode in ['taubin','central-diffusion']
O=R/('tree08-upper-volume-fairing-v1' if mode=='taubin' else 'tree08-upper-volume-fairing-v3-central')
assert not O.exists()
assert shutil.disk_usage(R).free>=10*1024**3
assert next(int(x.split()[1]) for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:'))>=6*1024**2
source=R/'tree08-connected-tip-cleanup-v2/candidate.npz'
a=np.load(source);v,f=a['vertices'],a['faces']
edges=np.unique(np.sort(np.concatenate([f[:,[0,1]],f[:,[1,2]],f[:,[2,0]]]),axis=1),axis=0)
rows,cols=np.r_[edges[:,0],edges[:,1]],np.r_[edges[:,1],edges[:,0]]
adj=coo_matrix((np.ones(len(rows)),(rows,cols)),shape=(len(v),len(v))).tocsr()
average=diags(1/np.asarray(adj.sum(axis=1)).ravel())@adj
s,c=np.sin(np.radians(35)),np.cos(np.radians(35));native_y=v@np.array([0,-s,-c])
blend=np.clip((315-native_y)/30,0,1);blend=blend*blend*(3-2*blend)
# The lower receiver-contact region is exact. Source-facing depth is inferred.
if mode=='central-diffusion':
    lateral=np.clip((v[:,0]-460)/55,0,1)*np.clip((690-v[:,0])/55,0,1)
    blend*=lateral*lateral*(3-2*lateral)
pins=blend==0
vertices=v.copy()
for step in range(160 if mode=='taubin' else 240):
    for gain in ([.4,-.401] if mode=='taubin' else [.4]):
        vertices+=gain*(average@vertices-vertices)*blend[:,None]
        delta=vertices-v;length=np.linalg.norm(delta,axis=1);over=length>18
        vertices[over]=v[over]+delta[over]*(18/length[over])[:,None]
        vertices[pins]=v[pins]
original=v[f];normal=np.cross(original[:,1]-original[:,0],original[:,2]-original[:,0]);area2=np.einsum('ij,ij->i',normal,normal)
delta=vertices-v;weight=np.ones(len(v));weight[pins]=0
origin=np.array([552.,-672.,235.])
for iteration in range(80):
    vertices=(v+delta*weight[:,None]-origin).astype(np.float32).astype(float)+origin
    vertices[weight<1e-3]=v[weight<1e-3]
    t=vertices[f];n=np.cross(t[:,1]-t[:,0],t[:,2]-t[:,0]);ratio=np.einsum('ij,ij->i',normal,n)/area2
    bad=ratio<.05
    if not bad.any():break
    weight[np.unique(f[bad])]*=.5
assert not bad.any()
assert np.array_equal(vertices[pins],v[pins])
check=audit(vertices,f)
assert not any(check[k] for k in ['nonmanifold_edges','winding_errors','zero_area'])
core=np.asarray(Image.open(R/'tree08-semantic-source-v1/bark-core-proposal.png'))>0
depth=native_depth([(vertices,f)]);lost=core&~np.isfinite(depth)
O.mkdir();np.savez_compressed(O/'candidate.npz',vertices=vertices,faces=f,before_vertices=v)
report=dict(status='PRIVATE_UPPER_VOLUME_TRIAL_REQUIRES_VISUAL_SOURCE_AND_INTERSECTION_REVIEW',source_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),candidate_sha256=hashlib.sha256((O/'candidate.npz').read_bytes()).hexdigest(),topology=check,source_pixels=int(core.sum()),source_pixels_lost=int(lost.sum()),fairing_mode=mode,lower_contact_vertices_exact=True,changed_vertices=int(np.any(vertices!=v,axis=1).sum()),max_displacement=float(np.linalg.norm(vertices-v,axis=1).max()),minimum_oriented_area_ratio=float(ratio.min()),orientation_damping_iterations=iteration,scope='Broad fairing of inferred upper wood; original source colors must be reprojected after coverage validation. No source color, strict crossing, or anatomy approval claimed.')
(O/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2),flush=True)
