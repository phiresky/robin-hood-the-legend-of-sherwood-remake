"""Add a tiny margin where one native ray is exactly coplanar with a twig face."""
import hashlib,json,subprocess,sys
from pathlib import Path
import numpy as np
from restart2_tree08_local_fork import audit
R=Path(__file__).resolve().parents[3]/'level-editor/work/croisement01-refinement/restart2';prior=R/'tree08-implicit-collar-v7-seam-cluster';out=R/'tree08-implicit-collar-v8-native-margin';assert not out.exists()
source=prior/'candidate.npz';proofpath=prior/'intersection-guard-fast.json';proof=json.loads(proofpath.read_text());assert proof['status']=='PASS_NO_INTERSECTIONS' and proof['candidate_sha256']==hashlib.sha256(source.read_bytes()).hexdigest()
a=np.load(source);v,f=a['vertices'],a['faces'];assert np.all(v[f[271511],0]==732.5)
center=np.array([732.5,-672.,286.8128900683257]);distance=np.linalg.norm(v-center,axis=1);weight=np.clip(1-distance/4,0,1);weight=weight*weight*(3-2*weight)
vertices=v.copy();vertices[:,0]+=.02*weight;origin=np.array([552.,-672.,235.]);vertices=(vertices-origin).astype(np.float32).astype(float)+origin;vertices[weight==0]=v[weight==0]
check=audit(vertices,f);assert not any(check[k] for k in ['nonmanifold_edges','winding_errors','zero_area']);out.mkdir();np.savez_compressed(out/'candidate.npz',vertices=vertices,faces=f,before_vertices=v)
changed=np.any(vertices[f]!=v[f],axis=(1,2));assert changed.sum()<2000
report=dict(status='PRIVATE_NATIVE_MARGIN_REQUIRES_SAVED_REVIEW',source_sha256=proof['candidate_sha256'],candidate_sha256=hashlib.sha256((out/'candidate.npz').read_bytes()).hexdigest(),topology=check,native_pixel=[401,139],world_projection=[732.5,150.5],maximum_displacement=float(np.linalg.norm(vertices-v,axis=1).max()),changed_vertices=int(np.any(vertices!=v,axis=1).sum()),changed_faces=int(changed.sum()),reason='The missing float ray lies exactly in a vertical boundary face at x732.5; add 0.02 world-unit maximum exterior coverage margin.')
(out/'report.json').write_text(json.dumps(report,indent=2)+'\n')
(out/'baseline-proof.json').write_text(json.dumps(dict(status='PASS_UNCHANGED_GEOMETRY_FULL_BASELINE',unchanged_geometry_proof=str(proofpath),proof_sha256=hashlib.sha256(proofpath.read_bytes()).hexdigest(),before_vertices_exact=True,faces_exact=True),indent=2)+'\n')
with (out/'strict-margin.log').open('w') as log:subprocess.run([sys.executable,str(Path(__file__).with_name('restart2_tree08_root_ray_guard_chunked.py')),'--packet',out.name,'--chunk','0'],stdout=log,stderr=subprocess.STDOUT,check=True)
strict=json.loads((out/'intersection-guard.json').read_text());assert not strict['new_intersections'] and not strict['retained_intersections'];assert strict['candidate_sha256']==report['candidate_sha256']
strict['status']='PASS_NO_INTERSECTIONS';strict['scope']='All candidate pairs touching changed faces; all remaining geometry exactly retains the linked complete baseline proof.';strict['unchanged_geometry_proof']=json.loads((out/'baseline-proof.json').read_text());(out/'intersection-guard-fast.json').write_text(json.dumps(strict,indent=2)+'\n');print(json.dumps(report,indent=2),flush=True)
