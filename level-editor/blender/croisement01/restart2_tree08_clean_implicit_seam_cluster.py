"""Consolidate the remaining microscopic stitch fan without changing its footprint."""
import hashlib,json
from pathlib import Path
import numpy as np
from restart2_tree08_local_fork import audit
R=Path(__file__).resolve().parents[3]/'level-editor/work/active/croisement01/restart2';prior=R/'tree08-implicit-collar-v6-seam-clean';out=R/'tree08-implicit-collar-v7-seam-cluster';assert not out.exists()
a=np.load(prior/'candidate.npz');v,f=a['vertices'],a['faces'];keep=167692;remove=[167690,167693]
assert np.linalg.norm(v[remove]-v[keep],axis=1).max()<.015
assert set(f[47283])=={4815,167690,167692} and set(f[332130])=={167690,167521,167693}
faces=f.copy()
for vertex in remove:faces[faces==vertex]=keep
repeated=(faces[:,0]==faces[:,1])|(faces[:,1]==faces[:,2])|(faces[:,2]==faces[:,0]);faces=faces[~repeated];used,remap=np.unique(faces,return_inverse=True);vertices,faces=v[used],remap.reshape(-1,3)
check=audit(vertices,faces);assert not any(check[k] for k in ['nonmanifold_edges','winding_errors','zero_area'])
out.mkdir();np.savez_compressed(out/'candidate.npz',vertices=vertices,faces=faces)
report=dict(status='PRIVATE_SEAM_FAN_REPAIRED_FULL_RECHECK_REQUIRED',source_sha256=hashlib.sha256((prior/'candidate.npz').read_bytes()).hexdigest(),candidate_sha256=hashlib.sha256((out/'candidate.npz').read_bytes()).hexdigest(),topology=check,removed_faces=np.flatnonzero(repeated).tolist(),collapsed_vertices=remove,retained_vertex=keep,maximum_displacement=float(np.linalg.norm(v[remove]-v[keep],axis=1).max()),position=v[keep].tolist(),limits='No full strict, source or contact claim until independent checks complete.')
(out/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2),flush=True)
