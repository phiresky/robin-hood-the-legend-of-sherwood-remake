"""Read-only classification of inherited Tree39 gray fallback surfaces."""
import hashlib,json,math,struct
from pathlib import Path
import numpy as np
ROOT=Path(__file__).resolve().parents[3]
BASE=ROOT/'level-editor/work/croisement02-refinement'
OUT=BASE/'tree39-gray-rim-audit-oct12-v1'
LIVE=ROOT/'level-editor/library/3d-assets/croisement02/croisement02-tree-39/model.glb'
APPROVED=BASE/'restart8-five-bark-approved-export-v1/tree-39-v1/delivery-v3/model.glb'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def read(p):
 raw=p.read_bytes();n=struct.unpack_from('<I',raw,12)[0];return json.loads(raw[20:20+n]),raw[20+n:]
def main():
 OUT.mkdir(exist_ok=True)
 g,tail=read(LIVE);a,original_tail=read(APPROVED)
 assert tail==original_tail
 changed=[i for i,(x,y)in enumerate(zip(g['materials'],a['materials']))if x!=y]
 assert changed==[4,5,6,7]
 assert {k:v for k,v in g.items()if k not in ('materials','textures')}=={k:v for k,v in a.items()if k not in ('materials','textures')}
 assert g['textures'][:len(a['textures'])]==a['textures']
 assert g['textures'][len(a['textures']):]==[{'sampler':0,'source':3}]
 binary=tail[8:]
 def acc(i):
  v=g['accessors'][i];b=g['bufferViews'][v['bufferView']];buf=binary if b.get('buffer',0)==0 else (LIVE.parent/g['buffers'][b['buffer']]['uri']).read_bytes();dt=np.dtype({5126:'<f4',5125:'<u4',5123:'<u2'}[v['componentType']]);dim={'VEC3':3,'VEC2':2,'SCALAR':1}[v['type']]
  return np.ndarray((v['count'],dim),dtype=dt,buffer=buf,offset=b.get('byteOffset',0)+v.get('byteOffset',0),strides=(b.get('byteStride',dt.itemsize*dim),dt.itemsize))
 rows=[]
 for mi,m in enumerate(g['materials']):
  if m.get('emissiveFactor')!=[0.18000000715255737]*3:continue
  for mesh in g['meshes']:
   for q in mesh['primitives']:
    if q['material']!=mi:continue
    v=acc(q['attributes']['POSITION']).astype(float)+[1699,-1110,0]
    xy=np.column_stack([v[:,0],-v[:,1]*math.sin(math.radians(35))-v[:,2]*math.cos(math.radians(35))])
    rows.append({'material_index':mi,'name':m['name'],'triangles':len(acc(q['indices']))//3,'native_projected_min':xy.min(0).tolist(),'native_projected_max':xy.max(0).tolist(),'z_minmax':[float(v[:,2].min()),float(v[:,2].max())],'material':m})
 refs=[LIVE,APPROVED,BASE/'restart8-five-bark-approved-export-v1/tree-39-v1/live-crown-sampler-guard-v1.json',BASE/'restart6-five-bark-fill-v1/tree-39-fill-v1/shader-restored-v3/ready-candidate-v1.json',BASE/'restart6-five-bark-fill-v1/tree-39-fill-v1/shader-restored-v3/close/textured.png',BASE/'approved-round-wood-integration-oct11-v1/editor-check-v2/browser-v1/croisement02-tree-38-root-contact-source.png']
 report={'status':'INHERITED_UNFILLED_APPEARANCE_NOT_EXPORT_REGRESSION','pins':{str(p):sha(p)for p in refs},'binary_geometry_uv_images_exact_to_approved_delivery':True,'all_wood_materials_exact':True,'only_material_changes':changed,'gray_regions':rows,'interpretation':'The lower rim is consistent with neutral untextured fallback on wood096/097. It is visible in the approved first-camera sheet and current contextual image. Constant-gray primitives are not sampled native gray/green source texture. Exact screen-pixel ray attribution has not been run.','prior_scope':'Approved bark card explicitly retained unsupported atlas locations using original material and made no hidden-surface completeness claim. Native gray/green/olive fragments are a separate protected category.','next_scoped_work':'If completing these unsupported regions, keep all geometry/UV/source materials and crown fixed; restrict new bark sampling to the three constant-gray primitive sets. Render close original/side/reverse views with only the two permitted Leicester references before synthesis. Existing approval does not justify painting native gray source texels.','canonical_writes':False,'private_model_correction':False,'reason_no_silent_repair':'Approved inherited unknown surfaces require a separately reviewable appearance completion; no evidence of integration corruption or misplaced geometry.'}
 (OUT/'diagnosis.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({'status':report['status'],'gray_triangles':sum(r['triangles']for r in rows),'report':str(OUT/'diagnosis.json')}))
if __name__=='__main__':main()
