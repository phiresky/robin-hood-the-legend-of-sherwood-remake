"""Trial small source-ray-only fairing of unapproved upper wood depth ridges."""
import hashlib,json
import numpy as np
from scipy.sparse import coo_matrix,diags
from PIL import Image
from restart2_tree08_root_embedding_cpu import R
from restart2_tree08_local_fork import audit
from restart2_tree08_junction_proof import native_depth
O=R/'tree08-upper-depth-fairing-cpu-v1';O.mkdir(exist_ok=False)
source=R/'tree08-bank-hug-affine-cpu-v4/candidate.npz';m=np.load(source);v=m['vertices'];f=m['faces'];s,c=np.sin(np.radians(35)),np.cos(np.radians(35));ray=np.array([0.,-c,s]);down=np.array([0.,-s,-c]);native_y=v@down
edges=np.unique(np.sort(np.concatenate([f[:,[0,1]],f[:,[1,2]],f[:,[2,0]]]),axis=1),axis=0);length=np.linalg.norm(v[edges[:,0]]-v[edges[:,1]],axis=1);pinned=np.zeros(len(v),bool);pinned[np.unique(edges[length<.003])]=True;active=(native_y<315)&~pinned
row=np.r_[edges[:,0],edges[:,1]];col=np.r_[edges[:,1],edges[:,0]];adj=coo_matrix((np.ones(len(row)),(row,col)),shape=(len(v),len(v))).tocsr();degree=np.asarray(adj.sum(axis=1)).ravel();assert np.all(degree>0);average=diags(1/degree)@adj;depth=v@ray;initial=depth.copy()
for iteration in range(10):
    for gain in [.3,-.31]:
        update=gain*(average@depth-depth);depth[active]+=update[active];depth=np.clip(depth,initial-2.,initial+2.);depth[~active]=initial[~active]
vertices=v+(depth-initial)[:,None]*ray;origin=np.array([552.,-672.,235.]);vertices=(vertices-origin).astype(np.float32).astype(float)+origin;vertices[~active]=v[~active];check=audit(vertices,f);before=native_depth([(v,f)]);after=native_depth([(vertices,f)]);core=np.asarray(Image.open(R/'tree08-semantic-source-v1/bark-core-proposal.png'))>0;lost=int((np.isfinite(before)&~np.isfinite(after)).sum());gained=int((~np.isfinite(before)&np.isfinite(after)).sum());np.savez_compressed(O/'candidate.npz',vertices=vertices,faces=f,before_vertices=v)
report=dict(status='CPU_INFERRED_DEPTH_TRIAL_NOT_REVIEW_READY',candidate_sha256=hashlib.sha256((O/'candidate.npz').read_bytes()).hexdigest(),parent_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),iterations=10,depth_displacement_max=float(abs(depth-initial).max()),source_core_depth_change=float(abs(after[core]-before[core]).max()),source_core_pixels=int(core.sum()),silhouette_lost=lost,silhouette_gained=gained,topology=check,lower_root_positions_exact=bool(np.array_equal(vertices[native_y>=315],v[native_y>=315])),pinned_tiny_edge_vertices=int(pinned.sum()),active_vertices=int(active.sum()),depth_laplacian_rms_before=float(np.mean((average@initial-initial)[active]**2)**.5),depth_laplacian_rms_after=float(np.mean((average@depth-depth)[active]**2)**.5),scope='Taubin fairing only along source rays, <=2worldunits, on unapproved upper depth. Native XY and lower bank-contact geometry frozen. This cannot establish source RGB/material preservation, strict intersections, or improved anatomy; all require independent checks before rendering/approval.')
(O/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2));assert lost==gained==0 and report['lower_root_positions_exact'];assert not any(check[k] for k in ['nonmanifold_edges','winding_errors','zero_area'])
