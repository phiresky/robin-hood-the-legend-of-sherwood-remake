"""Collapse one tiny stitch sliver identified by the complete strict audit."""
import hashlib,json
from pathlib import Path
import numpy as np
from restart2_tree08_local_fork import audit
R=Path(__file__).resolve().parents[3]/'level-editor/work/active/croisement01/restart2';prior=R/'tree08-implicit-collar-v5-clean';out=R/'tree08-implicit-collar-v6-seam-clean';assert not out.exists()
p=prior/'candidate.npz';proof=json.loads((prior/'intersection-guard-fast.json').read_text());assert proof['candidate_sha256']==hashlib.sha256(p.read_bytes()).hexdigest()
assert not proof['new_intersections'] and {tuple(x['faces']) for x in proof['retained_intersections']}=={(47283,332132),(47284,332132)}
a=np.load(p);v,f=a['vertices'],a['faces'];remove,keep=4810,167693
assert np.linalg.norm(v[remove]-v[keep])<.008
assert set(f[47283])=={167693,167694,4810} and set(f[47284])=={167691,167693,4810}
faces=f.copy();faces[faces==remove]=keep;repeat=(faces[:,0]==faces[:,1])|(faces[:,1]==faces[:,2])|(faces[:,2]==faces[:,0]);assert repeat.sum()==2
faces=faces[~repeat];used,remap=np.unique(faces,return_inverse=True);vertices,faces=v[used],remap.reshape(-1,3)
check=audit(vertices,faces);assert not any(check[k] for k in ['nonmanifold_edges','winding_errors','zero_area'])
out.mkdir();np.savez_compressed(out/'candidate.npz',vertices=vertices,faces=faces)
report=dict(status='PRIVATE_STITCH_SLIVER_REPAIRED_REQUIRES_FULL_RECHECK',source_sha256=proof['candidate_sha256'],candidate_sha256=hashlib.sha256((out/'candidate.npz').read_bytes()).hexdigest(),topology=check,removed_faces=np.flatnonzero(repeat).tolist(),collapsed_vertex=remove,retained_vertex=keep,maximum_displacement=float(np.linalg.norm(v[remove]-v[keep])),position=v[keep].tolist(),limits='This is a tiny upper/lower stitch correction. Source coverage, bank contact, all strict intersections and saved RGBA must be rechecked.')
(out/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2),flush=True)
