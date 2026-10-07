"""Restore two measured source-support pixels in the checked CPU sweep."""
import hashlib,json
from pathlib import Path
import numpy as np
from PIL import Image
from restart2_tree08_transport import strip_orientation
from restart2_tree08_transport_audit import coverage
R=Path(__file__).resolve().parents[3]/'level-editor/work/croisement01-refinement/restart2'
p=R/'tree08-v11-curvature-cpu-v5';out=R/'tree08-v11-curvature-cpu-v7';out.mkdir(exist_ok=False)
report=json.loads((p/'report.json').read_text());archive=np.load(p/'mesh.npz');payload={k:archive[k] for k in archive.files};sections=[];before=[];audits=[]
assert hashlib.sha256((p/'mesh.npz').read_bytes()).hexdigest()==report['mesh_sha256']
for section in report['mesh_sections']:
 i=section['index'];vertices=payload[f'vertices_{i}'];faces=payload[f'faces_{i}'];before.append(dict(vertices=vertices,faces=faces))
 if section['trace_id']==160:
  rings=vertices.reshape(-1,16,3);centers=rings.mean(1);radial=rings-centers[:,None,:];radius=np.linalg.norm(radial[:,0],axis=1)
  amount=3*np.exp(-((np.arange(len(rings))-25)/20)**2)
  vertices=(centers[:,None,:]+radial*(1+amount/radius)[:,None,None]).reshape(-1,3);payload[f'vertices_{i}']=vertices
 check=strip_orientation(vertices,16);assert check['nonoutward']==0;audits.append(dict(trace_id=section['trace_id'],**check));sections.append(dict(vertices=vertices,faces=faces))
old=coverage(before);hit=coverage(sections);core=np.asarray(Image.open(R/'tree08-semantic-source-v1/bark-core-proposal.png'))>0
assert not np.any(core&old&~hit),'Source support correction loses known wood'
assert all(hit[y-11,x-331] for x,y in report['lost_native_pixels'])
np.savez_compressed(out/'mesh.npz',**payload)
result=dict(status='CPU PASS for outward strips and preserved predecessor core coverage; model review HOLD',mesh_sha256=hashlib.sha256((out/'mesh.npz').read_bytes()).hexdigest(),parent_mesh_sha256=report['mesh_sha256'],mesh_sections=report['mesh_sections'],sections=audits,core_pixels=int(core.sum()),misses=int((core&~hit).sum()),miss_native_pixels=[[int(x)+331,int(y)+11] for y,x in np.argwhere(core&~hit)],newly_lost_pixels=[],source_repair=dict(trace_id=160,ring_center=25,gaussian_width=20,max_radius_expansion=3,restored_native_pixels=report['lost_native_pixels']),limitations=report['limitations'],recipe_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest())
(out/'report.json').write_text(json.dumps(result,indent=2)+'\n');print('CPU SOURCE SUPPORT PASS',result['misses'],'remaining prior core misses',flush=True)
