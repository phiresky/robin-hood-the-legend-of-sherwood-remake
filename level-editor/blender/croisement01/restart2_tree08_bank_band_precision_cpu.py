"""Remove one submillimetric root-band sliver without relaxing topology guards."""
import hashlib,json
import numpy as np
from PIL import Image
from restart2_tree08_root_embedding_cpu import R
from restart2_tree08_local_fork import audit
from restart2_tree08_junction_proof import native_depth
O=R/'tree08-bank-hug-affine-cpu-v3';O.mkdir(exist_ok=False)
source=R/'tree08-bank-hug-affine-cpu-v2/failed-mesh.npz';m=np.load(source);v=m['vertices'];f=m['faces'];before=m['before_vertices'];t=v[f];area=np.linalg.norm(np.cross(t[:,1]-t[:,0],t[:,2]-t[:,0]),axis=1);bad=np.flatnonzero(area<1e-9);assert len(bad)==1
face=f[bad[0]];edges=sorted((float(np.linalg.norm(v[a]-v[b])),int(a),int(b)) for a,b in zip(face,np.roll(face,1)));distance,keep,remove=edges[0];assert distance<.0003
s,c=np.sin(np.radians(35)),np.cos(np.radians(35));assert min(-v[[keep,remove],1]*s-v[[keep,remove],2]*c)>320
faces=f.copy();faces[faces==remove]=keep;valid=np.array([len(set(row))==3 for row in faces]);faces=faces[valid];assert int((~valid).sum())==2
# Retain the unreferenced vertex to preserve all other indices. Only the shorter
# sliver edge collapses; the surviving endpoint and every other position stay.
check=audit(v,faces);assert not any(check[k] for k in ['nonmanifold_edges','winding_errors','zero_area'])
original=np.load(R/'tree08-root-ray-cpu-v4/candidate.npz');old=native_depth([(original['vertices'],original['faces'])]);new=native_depth([(v,faces)]);core=np.asarray(Image.open(R/'tree08-semantic-source-v1/bark-core-proposal.png'))>0;assert np.array_equal(np.isfinite(old),np.isfinite(new));assert np.array_equal(old[core],new[core])
np.savez_compressed(O/'candidate.npz',vertices=v,faces=faces,before_vertices=before)
report=dict(status='CPU_BAND_SHEAR_PENDING_STRICT_CONTACT_INTERSECTIONS',candidate_sha256=hashlib.sha256((O/'candidate.npz').read_bytes()).hexdigest(),parent_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),collapsed_edge=dict(keep=keep,remove=remove,length=distance),removed_zero_index_faces=2,all_vertex_positions_unchanged_from_band_fit=True,source_core_pixels=int(core.sum()),source_core_depth_error=0.,silhouette_lost=0,silhouette_gained=0,topology=check,scope='Unapproved lower-root depth follows fitted global affine bands. One0.000242worldunit edge collapse removes float32 subthreshold sliver, without changing surviving vertex positions or relaxing audit tolerances; strict intersections/contact still pending.')
(O/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2))
