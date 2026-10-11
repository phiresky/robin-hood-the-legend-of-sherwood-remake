"""Freeze the gently faired join at saved Blender precision with native ray margin."""
import hashlib,json,shutil
from pathlib import Path
import numpy as np
from restart2_tree08_local_fork import audit
R=Path(__file__).resolve().parents[3]/'level-editor/work/croisement01-refinement/restart2';source=R/'tree08-implicit-collar-v17-gentle-seam/candidate.npz';out=R/'tree08-implicit-collar-v18-final-seam';assert not out.exists()
assert shutil.disk_usage(R).free>=10*1024**3
assert next(int(line.split()[1]) for line in Path('/proc/meminfo').read_text().splitlines() if line.startswith('MemAvailable:'))>=6*1024**2
a=np.load(source);v,f=a['vertices'],a['faces'];center=np.array([732.5,-672.,286.8128900683257]);distance=np.linalg.norm(v-center,axis=1);weight=np.clip(1-distance/4,0,1);weight=weight*weight*(3-2*weight)
vertices=v.copy();vertices[:,0]+=.02*weight;origin=np.array([552.,-672.,235.]);vertices=(vertices-origin).astype(np.float32).astype(float)+origin
check=audit(vertices,f);assert not any(check[k] for k in ['nonmanifold_edges','winding_errors','zero_area']);out.mkdir();np.savez_compressed(out/'candidate.npz',vertices=vertices,faces=f)
report=dict(status='PRIVATE_SAVED_PRECISION_CANDIDATE_RECHECK_REQUIRED',source_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),candidate_sha256=hashlib.sha256((out/'candidate.npz').read_bytes()).hexdigest(),topology=check,native_pixel=[401,139],maximum_margin_and_rounding=float(np.linalg.norm(vertices-v,axis=1).max()),custom_normals='Use exact --smooth-join-normals recipe from trial_solid_review helper after geometry creation.')
(out/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2),flush=True)
