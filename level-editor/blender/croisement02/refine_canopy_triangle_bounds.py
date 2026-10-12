"""Eliminate new broad-phase overlaps using saved triangle bounds, without claiming intersections."""
from pathlib import Path
import json,struct,hashlib
import numpy as np
from audit_canopy_static_clearance import primitives
R=Path.cwd();L=R/'level-editor/library';B=R/'level-editor/work/croisement02-refinement';a=B/'canopy-current-static-clearance-oct12-v1/audit.json';b=B/'canopy-current-static-clearance-refined-oct12-v1/audit.json';O=B/'canopy-new-neighbor-triangle-bounds-oct12-v1';O.mkdir(exist_ok=False);base=json.loads(a.read_text());refined=json.loads(b.read_text());scene=json.loads((L/'scenes/croisement02.rhlos-map.json').read_text());refs={x['id']:x for x in scene['assetSources']};results=[]
def load(path):
 raw=path.read_bytes();n=struct.unpack_from('<I',raw,12)[0];doc=json.loads(raw[20:20+n]);buffers=[raw[28+n:]]+[ (path.parent/x['uri']).read_bytes()for x in doc['buffers'][1:]]
 def read(i):
  x=doc['accessors'][i];v=doc['bufferViews'][x['bufferView']];dtype={5126:'<f4',5125:'<u4',5123:'<u2',5121:'u1'}[x['componentType']];width={'VEC3':3,'SCALAR':1}[x['type']];size=np.dtype(dtype).itemsize;return np.ndarray((x['count'],width),dtype=dtype,buffer=buffers[v.get('buffer',0)],offset=v.get('byteOffset',0)+x.get('byteOffset',0),strides=(v.get('byteStride',size*width),size))
 return doc,read,hashlib.sha256(raw).hexdigest()
for item in refined['crowns']:
 if not item['new_possible_neighbor_overlaps']:continue
 original=next(x for x in base['crowns']if x['asset']==item['asset']);moving,read,digest=load(Path(original['candidate_pin']['path']));assert digest==original['candidate_pin']['sha256'];placement=next(x for x in scene['placements']if x['id']==item['asset']);meshes=[(p,w)for _,_,p,w,_ in primitives(moving,placement['transform'],placement.get('parts'))if p.get('targets')]
 for neighbor in item['new_possible_neighbor_overlaps']:
  aid=neighbor['asset'];static,sread,sdigest=load(L/refs[aid]['model']);assert sdigest==refs[aid]['model_sha256'];sp=next(x for x in scene['placements']if x['id']==neighbor['placement']);_,_,prim,world,_=next(x for x in primitives(static,sp['transform'],sp.get('parts'))if x[0]==neighbor['node']and x[1]==neighbor['primitive']);vertices=sread(prim['attributes']['POSITION'])@world[:3,:3].T+world[:3,3];tris=vertices[sread(prim['indices']).reshape(-1,3)];slo=tris.min(1)-1e-5;shi=tris.max(1)+1e-5;global_lo=slo.min(0);global_hi=shi.max(0);phases=[]
  for phase in range(14):
   pairs=0;examples=[];tested=0
   for mindex,(p,w)in enumerate(meshes):
    local=read(p['attributes']['POSITION']);local=local+read(p['targets'][phase-1]['POSITION'])if phase else local;points=local@w[:3,:3].T+w[:3,3];tri=points[read(p['indices']).reshape(-1,3)];lo=tri.min(1)-1e-5;hi=tri.max(1)+1e-5;ids=np.flatnonzero(((lo<=global_hi)&(hi>=global_lo)).all(1));tested+=len(ids)
    for start in range(0,len(ids),16):
     q=ids[start:start+16];overlap=((lo[q,None]<=shi[None])&(hi[q,None]>=slo[None])).all(2);pairs+=int(overlap.sum())
     for x,y in np.argwhere(overlap)[:max(0,10-len(examples))]:examples.append({'moving_primitive':mindex,'moving_triangle':int(q[x]),'static_triangle':int(y)})
   phases.append({'phase':phase,'moving_triangles_after_global_prune':tested,'triangle_bound_overlap_pairs':pairs,'first_pairs':examples})
  result={'moving_asset':item['asset'],'static_asset':aid,'static_node':neighbor['node'],'static_primitive':neighbor['primitive'],'static_model_sha256':sdigest,'phases':phases,'separated_all_phases':all(x['triangle_bound_overlap_pairs']==0 for x in phases)};results.append(result);print(item['asset'],aid,'pairs',sum(x['triangle_bound_overlap_pairs']for x in phases),flush=True)
report={'status':'TRIANGLE_BOUND_CLEARANCE_REFINEMENT','inputs':{str(p):hashlib.sha256(p.read_bytes()).hexdigest()for p in [a,b]},'pairs':results,'limits':['Positive triangle-bound separation is sufficient for geometric separation of those saved surfaces at evaluated14STEPposes.','Triangle-bound overlap is not a surface intersection; exact triangle test/contact review still required.','Only six newly implicated primitive pairs are evaluated. Existing baseline overlaps and moving-neighbor combinations remain unproved.']};(O/'audit.json').write_text(json.dumps(report,indent=2)+'\n');print('sha',hashlib.sha256((O/'audit.json').read_bytes()).hexdigest())
