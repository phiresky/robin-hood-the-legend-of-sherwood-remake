"""Freeze independent room-cover source ownership and initial/applied semantics."""
import hashlib,json
from pathlib import Path
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
W=ROOT/'level-editor/work/active/york';OUT=W/'restart2/restart61-room-source-v1'
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
manifest=W/'geometry-pass-01/native-state-source-v1/manifest.json'
assert sha(manifest)=='996e048c12c2326d059c6665ff70bda548937e2d06aa87c8d3f898eee0525cfe'
m=json.loads(manifest.read_text());r=next(r for r in m['records'] if r['id']=='patch-005');rec=r['record']
assert rec['definitive'] and not rec['integrate_in_background'] and not rec['transition_animation_valid'] and not rec['end_animation_valid']
frame=manifest.parent/r['rows'][0]['frames'][0]['image'];assert sha(frame)==r['rows'][0]['frames'][0]['sha256']
level=W/'baseline/york.rhp.json';data=json.loads(level.read_text());masks=[]
for kind in ['old_masks','new_masks']:
 for ref in rec[kind]:
  matches=[(i,v) for i,v in enumerate(data['masks']) if v['layer']==ref['layer']];i,v=matches[ref['index']];p=W/f'baseline/masks/{i:06d}.png';im=Image.open(p).convert('L')
  masks.append({'phase':kind,'global_id':i,'layer_local':ref,'source_record':v,'path':str(p.relative_to(ROOT)),'file_sha256':sha(p),'pixels_sha256':hashlib.sha256(im.tobytes()).hexdigest(),'size':list(im.size)})
report={'status':'SOURCE_FROZEN_NOT_GEOMETRY_APPROVAL','native_manifest_sha256':sha(manifest),'level_sha256':sha(level),'patch':r,'frame_path':str(frame.relative_to(ROOT)),'frame_sha256':sha(frame),'masks':masks,'state_effect':{'initial':'Opaque source-supported stone cover visible; native old masks active.','apply':'Immediate definitive disappearance; no transition or final sprite, new masks active. No static artwork rewrite.','reset':'Initial cover restored independently of gate and winch.','physical_applied':'No cover object. Existing room receivers remain context and require independent ownership/appearance approval.','independent_from':['patch-000','patch-004']},'geometry_inference':{'floor_game_z':90.001,'rationale':'Adjacent room floor hypothesis retained from source-backed winch support contact; hidden wall depth is not observable from this artwork.','status':'Pending geometry/contact review; not yet ready.'}}
(OUT/'source.json').write_text(json.dumps(report,indent=2)+'\n');print(OUT/'source.json')
