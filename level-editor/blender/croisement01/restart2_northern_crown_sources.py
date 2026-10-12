"""Inventory isolated animated crowns separately from background wood and occlusion masks."""
from pathlib import Path
import hashlib
import json
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
r=ROOT/'level-editor/work/active/croisement01';out=r/'restart2/northern-native-crown-sources-v1';out.mkdir(exist_ok=False)
source=json.loads((r/'animation-references/manifest.json').read_text())
audit=json.loads((r/'restart2/native-controls-private-oct11-v2/animation-frame-audit.json').read_text())
pins={(x['animation'],x['frame']):x for x in audit['records']}
meta=json.loads((r/'baseline/masks/manifest.json').read_text());masks=meta['masks']
width,height=Image.open(r/'baseline/covered.png').size
rows=[]
for animation in source['animations'][:5]:
 index=animation['index'];native=next(x for x in masks if x['index']==93+index)
 union=np.zeros((height,width),bool);records=[];first=None
 for number,frame in enumerate(animation['frames']):
  p=Path(frame['image']);digest=hashlib.sha256(p.read_bytes()).hexdigest();assert digest==pins[index,number]['converted_sha256']
  rgba=np.asarray(Image.open(p).convert('RGBA'));x,y,w,h=frame['bbox'];assert rgba.shape==(h,w,4)
  global_alpha=np.zeros_like(union);lx,ty,rx,by=max(0,x),max(0,y),min(width,x+w),min(height,y+h)
  global_alpha[ty:by,lx:rx]=rgba[ty-y:by-y,lx-x:rx-x,3]>0;union|=global_alpha
  if number==0:first=global_alpha
  records.append(dict(frame=number,path=str(p.relative_to(ROOT)),sha256=digest,bbox=frame['bbox'],duration_ticks=frame['delay']+1,opaque_pixels=int((rgba[:,:,3]>0).sum())))
 native_alpha=np.zeros_like(union);x,y=native['box_top_left'];w,h=native['box_size'];native_alpha[y:y+h,x:x+w]=np.asarray(Image.open(r/'baseline/masks'/native['png']).convert('L'))>0
 wood=[]
 for m in masks:
  if not(0<=m['index']<=23):continue
  x,y=m['box_top_left'];w,h=m['box_size'];a=np.asarray(Image.open(r/'baseline/masks'/m['png']).convert('L'))>0
  overlap=int(np.count_nonzero(union[y:y+h,x:x+w]&a))
  if overlap:wood.append(dict(mask=m['index'],animated_leaf_overlap_pixels=overlap))
 rows.append(dict(animation=index,profile=animation['profile'],sprite=animation['sprite'],native_occlusion_mask=93+index,
   frames=records,loop_ticks=sum(f['duration_ticks'] for f in records),first_frame_pixels=int(first.sum()),animated_union_pixels=int(union.sum()),
   native_mask_pixels=int(native_alpha.sum()),mask_without_animated_leaf_pixels=int((native_alpha&~union).sum()),
   animated_leaf_outside_mask_pixels=int((union&~native_alpha).sum()),overlapping_wood_domains=wood,
   ownership='Native RGBA defines leaves; background wood stays underneath. Overlap does not establish shared physical tree ownership.'))
report=dict(status='EXACT_NATIVE_SOURCE_INVENTORY_NOT_GEOMETRY_APPROVAL',crowns=rows,
 limitations=['Physical crown-to-trunk association, inferred off-map completion, native sorting and saved-model appearance require individual review.',
              'Occlusion masks are not isolated texture alpha; do not sample covered-map background through these masks as foliage.'])
(out/'inventory.json').write_text(json.dumps(report,indent=2)+'\n')
for row in rows:print({k:row[k] for k in ('animation','profile','first_frame_pixels','animated_union_pixels','mask_without_animated_leaf_pixels')})
