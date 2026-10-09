"""Private source-ray depth refit for unapproved lower roots against the real bank."""
import hashlib,json
import numpy as np
from scipy.interpolate import RBFInterpolator
from PIL import Image
from restart2_tree08_root_embedding_cpu import R
from restart2_tree08_junction_proof import native_depth
from restart2_tree08_local_fork import audit

O=R/'tree08-bank-hug-cpu-v2';O.mkdir(exist_ok=False)
source=R/'tree08-root-ray-cpu-v4/candidate.npz'
mesh=np.load(source);original=mesh['vertices'];faces=mesh['faces']
s,c=np.sin(np.radians(35)),np.cos(np.radians(35));ray=np.array([0.,-c,s]);down=np.array([0.,-s,-c]);xy=np.c_[original[:,0],original@down]
route=json.loads((R/'tree08-wood-prototype-v14-root-ray/current-contact/root-route-visibility.json').read_text())
# Prior depths were hypotheses, not observed artwork. Keep the source projection
# while bringing each root front toward the actual source-ray receiver surface.
anchors=np.array([[v['native'][0]+.5,v['native'][1]+.5] for v in route['samples']]);values=np.array([2.5-v['ray_clearance'] for v in route['samples']])
unique,inverse=np.unique(anchors,axis=0,return_inverse=True);combined=np.array([values[inverse==i].mean() for i in range(len(unique))]);assert max(np.ptp(values[inverse==i]) for i in range(len(unique)))<1e-6;anchors,values=unique,combined
rbf=RBFInterpolator(anchors,values,kernel='thin_plate_spline',smoothing=.15)
selected=xy[:,1]>320;delta=np.zeros(len(original));delta[selected]=np.clip(rbf(xy[selected]),-35.,5.)
t=np.clip((xy[:,1]-320)/25,0,1);delta*=t*t*(3-2*t)
vertices=original+delta[:,None]*ray;origin=np.array([552.,-672.,235.]);vertices=(vertices-origin).astype(np.float32).astype(float)+origin;vertices[~selected]=original[~selected]
projection_error=float(np.max(abs(np.c_[vertices[:,0],vertices@down]-xy)))
before=native_depth([(original,faces)]);after=native_depth([(vertices,faces)]);core=np.asarray(Image.open(R/'tree08-semantic-source-v1/bark-core-proposal.png'))>0;finite=np.isfinite(before);lost=int((finite&~np.isfinite(after)).sum());gained=int((~finite&np.isfinite(after)).sum());core_error=float(abs(before[core]-after[core]).max())
check=audit(vertices,faces);np.savez_compressed(O/'candidate.npz',vertices=vertices,faces=faces,before_vertices=original)
report=dict(status='CPU_CANDIDATE_PENDING_STRICT_CONTACT_INTERSECTION_AND_ANATOMY',candidate_sha256=hashlib.sha256((O/'candidate.npz').read_bytes()).hexdigest(),parent_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),projection_vertex_error=projection_error,source_core_pixels=int(core.sum()),source_core_depth_error=core_error,silhouette_lost=lost,silhouette_gained=gained,topology=check,moved_vertices=int((delta!=0).sum()),ray_delta_range=[float(delta.min()),float(delta.max())],anchor_fit_error=float(abs(rbf(anchors)-values).max()),scope='Only unapproved inferred lower-root depth. Native XY and upper core retained. Real receiver unchanged. No texture/RGB, full contact, strict intersections, or visual quality claim until independently checked.')
(O/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2));assert projection_error<.0001 and core_error==0 and lost==gained==0;assert not any(check[k] for k in ['nonmanifold_edges','winding_errors','zero_area'])
