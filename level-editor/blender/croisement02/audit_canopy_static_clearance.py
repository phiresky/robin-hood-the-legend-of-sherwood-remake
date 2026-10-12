"""Conservative saved-accessor bounds audit; overlaps never certify contact."""
from pathlib import Path
import json,struct,hashlib,itertools,math
import numpy as np
from scipy.spatial.transform import Rotation
R=Path.cwd();L=R/'level-editor/library';B=R/'level-editor/work/croisement02-refinement';O=B/'canopy-current-static-clearance-oct12-v1';S=math.sin(math.radians(35))
def sha(p):
 h=hashlib.sha256()
 with p.open('rb')as f:
  for c in iter(lambda:f.read(1048576),b''):h.update(c)
 return h.hexdigest()
def header(p):
 with p.open('rb')as f:
  f.seek(12);n=struct.unpack('<I',f.read(4))[0];f.seek(20);return json.loads(f.read(n))
def primitives(doc,placement,parts=None):
 matrix=np.eye(4);matrix[:3,3]=[placement['dx'],placement['dz'],placement['dy']/S];assert placement['rot_deg']==0
 def visit(i,parent,hidden=False):
  node=doc['nodes'][i];ex=node.get('extras',{});part=ex.get('editor_part_node',ex.get('source_node'));settings=(parts or{}).get(part,{})
  hidden=hidden or settings.get('hidden',ex.get('default_hidden',False))
  if 'matrix'in node:local=np.asarray(node['matrix']).reshape(4,4).T
  else:
   local=np.eye(4);local[:3,:3]=Rotation.from_quat(node.get('rotation',[0,0,0,1])).as_matrix()@np.diag(node.get('scale',[1,1,1]));local[:3,3]=node.get('translation',[0,0,0])
  world=parent@local
  if 'mesh'in node and not hidden:
   for j,p in enumerate(doc['meshes'][node['mesh']]['primitives']):yield i,j,p,world,ex
  for c in node.get('children',[]):yield from visit(c,world,hidden)
 for i in doc['scenes'][doc.get('scene',0)]['nodes']:yield from visit(i,matrix)
def bounds(doc,prim,world,phase=0):
 a=doc['accessors'][prim['attributes']['POSITION']];lo=np.asarray(a['min'],float);hi=np.asarray(a['max'],float)
 if phase:
  a=doc['accessors'][prim['targets'][phase-1]['POSITION']];lo=lo+np.asarray(a['min']);hi=hi+np.asarray(a['max'])
 corners=np.array(list(itertools.product(*zip(lo,hi))));points=corners@world[:3,:3].T+world[:3,3];return np.array([points.min(0),points.max(0)])
def main():
 O.mkdir(exist_ok=False);map_path=L/'scenes/croisement02.rhlos-map.json';map_hash=sha(map_path);scene=json.loads(map_path.read_text());refs={x['id']:x for x in scene['assetSources']};static=[];resource_pins={str(map_path):map_hash}
 for placement in scene['placements']:
  for aid in placement['assets']:
   ref=refs[aid];path=L/ref['model'];digest=sha(path);assert digest==ref['model_sha256'],aid;resource_pins[str(path)]=digest;doc=header(path)
   for i,j,p,w,ex in primitives(doc,placement['transform'],placement.get('parts')):static.append({'asset':aid,'placement':placement['id'],'node':i,'primitive':j,'role':ex.get('projection_component','wood-or-other'),'bounds':bounds(doc,p,w).tolist()})
 for ref in scene['sceneAssets']:
  path=L/ref['model'];assert sha(path)==ref['model_sha256'];resource_pins[str(path)]=ref['model_sha256'];doc=header(path)
  for i,j,p,w,ex in primitives(doc,{'dx':0,'dy':0,'dz':0,'rot_deg':0}):static.append({'asset':ref['id'],'placement':None,'node':i,'primitive':j,'role':'scene-ground','bounds':bounds(doc,p,w).tolist()})
 boxes=np.asarray([x['bounds']for x in static]);results=[]
 for group in [0,2,3,4,5,6,7]:
  path=B/(f'canopy{group:02}-complete-installed-oct12-v1/report.json'if group in [3,4]else f'restart99-canopy{group:02}-combined-v1/report.json');bundle=json.loads(path.read_text());resource_pins[str(path)]=sha(path)
  for member in bundle['assets']:
   aid=member['asset_id'];pin=member['source_model'];candidate=Path(pin['path']);assert sha(candidate)==pin['sha256'];resource_pins[str(candidate)]=pin['sha256'];source=member['static_source'];assert sha(Path(source['path']))==source['sha256'],'Static base changed '+aid;current=next(p for p in scene['placements']if p['id']==aid);assert current==member['placement'],'Placement changed '+aid
   doc=header(candidate);crowns=[(p,w)for i,j,p,w,ex in primitives(doc,current['transform'],current.get('parts'))if p.get('targets')];assert crowns
   phases=[]
   for phase in range(14):
    bs=np.array([bounds(doc,p,w,phase)for p,w in crowns]);bb=np.array([bs[:,0].min(0),bs[:,1].max(0)]);gap=np.maximum(boxes[:,0]-bb[1],bb[0]-boxes[:,1]);separated=(gap>1e-4).any(1);overlap=[i for i in np.flatnonzero(~separated)if static[i]['asset']!=aid];phases.append({'phase':phase,'bounds':bb.tolist(),'overlap_static_primitive_indices':list(map(int,overlap)),'own_wood_overlap_indices':[int(i)for i in np.flatnonzero(~separated)if static[i]['asset']==aid and static[i]['role']!='crown']})
   basis=set(phases[0]['overlap_static_primitive_indices']);new=sorted(set().union(*(set(p['overlap_static_primitive_indices'])for p in phases[1:]))-basis)
   results.append({'group':group,'asset':aid,'basis_pin':source,'candidate_pin':pin,'phases':phases,'new_possible_neighbor_overlaps':[{'index':i,**static[i]}for i in new]});print(group,aid,'new-bounds-overlaps',len(new),flush=True)
 assert sha(map_path)==map_hash
 for path,digest in resource_pins.items():assert sha(Path(path))==digest,'Input drift '+path
 report={'status':'CONSERVATIVE_DECLARED_BOUND_SEPARATION_AUDIT_NOT_CONTACT_CERTIFICATION','scope':'Seven complete unapproved groups,43crowns,14phases against current visible static model primitives. Group1 approved Tree42 requires separate placement contract and remains explicitly excluded pending that binding.','map':{'path':str(map_path),'sha256':map_hash},'static_primitives':static,'crowns':results,'resource_pins':resource_pins,'limits':['Accessor min/max boxes are conservative under morph addition and node transforms. Positive axis gap >0.0001 proves declared-bound separation only.','Bounds overlaps are candidates, not intersections or attachment evidence. Alpha/source ownership, moving-neighbor interaction, and triangle self-intersection are not certified.','Hidden static parts excluded by saved placement overrides/default_hidden. Unmodeled dynamic states are not covered.']};(O/'audit.json').write_text(json.dumps(report,indent=2)+'\n');print('audit',sha(O/'audit.json'))

if __name__=='__main__':main()
