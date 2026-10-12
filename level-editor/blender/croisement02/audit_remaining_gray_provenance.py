"""Localize inherited constant fallback primitives without rendering or editing models."""
import hashlib,json,math,struct
from pathlib import Path
import numpy as np
ROOT=Path(__file__).resolve().parents[3]
BASE=ROOT/'level-editor/work/croisement02-refinement'
def sha(p):
 h=hashlib.sha256()
 with p.open('rb') as f:
  for c in iter(lambda:f.read(1048576),b''):h.update(c)
 return h.hexdigest()
def read(p):
 raw=p.read_bytes();n=struct.unpack_from('<I',raw,12)[0];return json.loads(raw[20:20+n]),raw[20+n:]
def main():
 rows=[]
 for n in (18,19,25):
  folder=BASE/(f'restart8-five-bark-approved-export-v1/tree-{n}-v1' if n==18 else f'restart25-approved-appearance-delta-v1/tree-{n}-v1')
  ep=folder/'export.json';e=json.loads(ep.read_text());p=ROOT/f'level-editor/library/3d-assets/croisement02/croisement02-tree-{n}/model.glb';g,tail=read(p)
  a,oldtail=read(folder/'delivery-v4/model.glb')
  def acc(i):
   v=g['accessors'][i];b=g['bufferViews'][v['bufferView']];buf=tail[8:] if b.get('buffer',0)==0 else (p.parent/g['buffers'][b['buffer']]['uri']).read_bytes();dt=np.dtype({5126:'<f4',5125:'<u4',5123:'<u2'}[v['componentType']]);dim={'VEC3':3,'VEC2':2,'SCALAR':1}[v['type']]
   return np.ndarray((v['count'],dim),dtype=dt,buffer=buf,offset=b.get('byteOffset',0)+v.get('byteOffset',0),strides=(b.get('byteStride',dt.itemsize*dim),dt.itemsize))
  regions=[]
  for mesh in g['meshes']:
   for q in mesh['primitives']:
    mi=q.get('material');m=g['materials'][mi]
    if m.get('emissiveFactor')!=[0.18000000715255737]*3:continue
    assert 'emissiveTexture' not in m and 'baseColorTexture' not in m.get('pbrMetallicRoughness',{})
    pos=acc(q['attributes']['POSITION']).astype(float);ids=acc(q['indices']).ravel();v=pos[ids]+e['pivot'];tri=v.reshape(-1,3,3);area=np.linalg.norm(np.cross(tri[:,1]-tri[:,0],tri[:,2]-tri[:,0]),axis=1)*.5
    xy=np.column_stack([v[:,0],-v[:,1]*math.sin(math.radians(35))-v[:,2]*math.cos(math.radians(35))])
    regions.append({'material':m['name'],'material_index':mi,'triangles':len(tri),'surface_area':float(area.sum()),'native_projected_min':xy.min(0).tolist(),'native_projected_max':xy.max(0).tolist(),'world_min':v.min(0).tolist(),'world_max':v.max(0).tolist(),'near_ground_triangles_all_vertices_below_z1':int((tri[:,:,2].max(1)<1).sum()),'material_exact_to_delivery_v4':m==a['materials'][mi]})
  changes=[k for k in set(g)|set(a) if g.get(k)!=a.get(k)]
  rows.append({'tree':n,'model':str(p),'model_sha256':sha(p),'export_record':str(ep),'export_record_sha256':sha(ep),'approved_worker':e['source_model'],'approved_worker_sha256':e['source_sha256'],'delivery_v4_sha256':sha(folder/'delivery-v4/model.glb'),'binary_exact_to_delivery_v4':tail==oldtail,'json_fields_changed_from_delivery_v4':changes,'constant_shader_leaves_in_approved_export':[{'object':r['object'],'leaves':{k:v for k,v in r.get('leaves',{}).items() if v[0]=='constant'}} for r in e['conversion_records'] if any(v[0]=='constant' for v in r.get('leaves',{}).values())],'regions':regions})
 report={'status':'INHERITED_UNSUPPORTED_WOOD_APPEARANCE_CONFIRMED','assets':rows,'visibility_limits':'Bounding boxes localize surfaces, not screen visibility. Earlier approved close sheets were reviewed; no quantitative ray visibility claim. Textured gray/olive pixels remain protected. Hidden reusable surfaces remain completion scope.','next_scope':'Same geometry-preserving constant-primitive-only texture completion as Tree39, with separate grouped input review; no synthesis or canonical writes performed.'}
 out=BASE/'tree-gray-material-audit-oct12-v1/remaining-provenance.json';out.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2))
if __name__=='__main__':main()
