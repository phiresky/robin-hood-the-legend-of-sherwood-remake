"""Refine broad-phase candidates with actual saved morph vertex bounds."""
from pathlib import Path
import json,struct,hashlib
import numpy as np
from audit_canopy_static_clearance import primitives
R=Path.cwd();B=R/'level-editor/work/active/croisement02';SOURCE=B/'canopy-current-static-clearance-oct12-v1/audit.json';O=B/'canopy-current-static-clearance-refined-oct12-v1';O.mkdir(exist_ok=False);report=json.loads(SOURCE.read_text());scene=json.loads((R/'level-editor/library/scenes/croisement02.rhlos-map.json').read_text());static=report['static_primitives'];boxes=np.asarray([x['bounds']for x in static]);results=[]
for item in report['crowns']:
 path=Path(item['candidate_pin']['path']);raw=path.read_bytes();assert hashlib.sha256(raw).hexdigest()==item['candidate_pin']['sha256'];n=struct.unpack_from('<I',raw,12)[0];doc=json.loads(raw[20:20+n]);data=raw[28+n:];assert len(doc['buffers'])==1
 def read(i):
  a=doc['accessors'][i];v=doc['bufferViews'][a['bufferView']];assert a['componentType']==5126 and a['type']=='VEC3';return np.ndarray((a['count'],3),dtype='<f4',buffer=data,offset=v.get('byteOffset',0)+a.get('byteOffset',0),strides=(v.get('byteStride',12),4))
 placement=next(p for p in scene['placements']if p['id']==item['asset']);crowns=[(p,w)for _,_,p,w,_ in primitives(doc,placement['transform'],placement.get('parts'))if p.get('targets')];phases=[]
 for phase in range(14):
  hits=set();own=set();all_bounds=[]
  for p,w in crowns:
   local=read(p['attributes']['POSITION']);local=local+read(p['targets'][phase-1]['POSITION'])if phase else local;world=local@w[:3,:3].T+w[:3,3];bb=np.array([world.min(0),world.max(0)]);all_bounds.append(bb.tolist());gap=np.maximum(boxes[:,0]-bb[1],bb[0]-boxes[:,1]);overlap=np.flatnonzero(~(gap>1e-4).any(1));hits.update(int(i)for i in overlap if static[i]['asset']!=item['asset']);own.update(int(i)for i in overlap if static[i]['asset']==item['asset']and static[i]['role']!='crown')
  phases.append({'phase':phase,'primitive_exact_vertex_bounds':all_bounds,'possible_neighbors':sorted(hits),'own_wood_bounds_overlap':sorted(own)})
 basis=set(phases[0]['possible_neighbors']);new=sorted(set().union(*(set(x['possible_neighbors'])for x in phases[1:]))-basis);results.append({'asset':item['asset'],'group':item['group'],'phases':phases,'new_possible_neighbor_overlaps':[{'index':i,**static[i]}for i in new]});print(item['asset'],len(new),flush=True)
result={'status':'EXACT_MOVED_VERTEX_PRIMITIVE_BOUND_AUDIT_NOT_TRIANGLE_CONTACT_PROOF','source_audit':{'path':str(SOURCE),'sha256':hashlib.sha256(SOURCE.read_bytes()).hexdigest()},'crowns':results,'limits':['Actual saved position+morph vertices used for each separate crown primitive; static bounds remain declared accessor bounds.','Positive gap proves separation of those bounded surfaces. Overlaps still require triangle/visual review; no penetration or attachment claim.','Group1 Tree42 remains separate pending source placement proof.']};(O/'audit.json').write_text(json.dumps(result,indent=2)+'\n');print('sha',hashlib.sha256((O/'audit.json').read_bytes()).hexdigest())
