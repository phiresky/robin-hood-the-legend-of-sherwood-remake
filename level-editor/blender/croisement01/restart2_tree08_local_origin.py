"""CPU proof of local float32 operands with an explicit unchanged world frame."""
import hashlib,json
from pathlib import Path
import numpy as np
from PIL import Image
from restart2_tree08_transport import strip_orientation
from restart2_tree08_transport_audit import coverage
R=Path(__file__).resolve().parents[3]/'level-editor/work/croisement01-refinement/restart2';p=R/'tree08-v12-chain-cpu-v3';out=R/'tree08-v12-local-origin-v1';out.mkdir(exist_ok=False);r=json.loads((p/'report.json').read_text());a=np.load(p/'mesh.npz');all_vertices=np.concatenate([a[f"vertices_{s['index']}"] for s in r['mesh_sections']]);origin=np.round((all_vertices.min(0)+all_vertices.max(0))/2);payload={};sections=[];records=[];s,c=np.sin(np.radians(35)),np.cos(np.radians(35))
for section in r['mesh_sections']:
 i=section['index'];world=a[f'vertices_{i}'];local=(world-origin).astype(np.float32);restored=local.astype(np.float64)+origin;faces=a[f'faces_{i}'];payload[f'vertices_{i}']=local;payload[f'faces_{i}']=faces;displacement=restored-world;native=np.column_stack((displacement[:,0],-displacement[:,1]*s-displacement[:,2]*c));check=strip_orientation(local.astype(float),16);assert check['nonoutward']==0;records.append(dict(trace_id=section['trace_id'],world_max_error=float(np.linalg.norm(displacement,axis=1).max()),source_max_pixel_error=float(np.linalg.norm(native,axis=1).max()),uv_max_error=float(np.abs(native/np.array([446,461])).max()),**check));sections.append(dict(vertices=restored,faces=faces))
core=np.asarray(Image.open(R/'tree08-semantic-source-v1/bark-core-proposal.png'))>0;hit=coverage(sections);assert not np.any(core&~hit),'Local float32 operands lose source core';np.savez_compressed(out/'mesh-local.npz',**payload)
result=dict(status='CPU local-origin/source-frame PASS; Boolean construction not yet retried',parent_mesh_sha256=r['mesh_sha256'],local_mesh_sha256=hashlib.sha256((out/'mesh-local.npz').read_bytes()).hexdigest(),world_origin=origin.tolist(),restore='Keep mesh vertices local; set object matrix_world translation to world_origin. Source projection and world-space checks must apply that matrix.',max_world_quantization_error=max(x['world_max_error'] for x in records),max_source_pixel_error=max(x['source_max_pixel_error'] for x in records),max_uv_error=max(x['uv_max_error'] for x in records),source_core_pixels=int(core.sum()),source_core_misses=0,nonoutward=sum(x['nonoutward'] for x in records),mesh_sections=r['mesh_sections'],sections=records)
assert result['max_world_quantization_error']<.00005
(out/'report.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps({k:v for k,v in result.items() if k not in ['sections','mesh_sections']},indent=2))
