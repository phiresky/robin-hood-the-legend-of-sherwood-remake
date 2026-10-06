"""Inventory every residual neutral flat-floor texel without assigning source ownership."""
import json, hashlib
from pathlib import Path
from collections import defaultdict
import numpy as np
from PIL import Image, ImageDraw
from scipy import ndimage
ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/croisement02-refinement'
D=OUT/'restart4-ground-closure-inventory-v1'
def sha(p): return hashlib.sha256(p.read_bytes()).hexdigest()
def read(p): return json.loads(p.read_text())
def mask(p): return np.array(Image.open(p).convert('L'))>0
def main():
 D.mkdir(exist_ok=False)
 base=OUT/'restart4-remaining-floor-bake-v1'
 atlas=np.array(Image.open(base/'composite.png').convert('RGBA'))
 known=mask(base/'known-native-domain.png')
 prep=OUT/'restart2-ground-completion/preparation-v1'
 relief=mask(prep/'separate_relief.png'); deferred=mask(prep/'deferred_state_floor.png')
 gray=np.all(atlas[:,:,:3]==127,axis=2); remaining=gray&~known&~relief
 assert remaining.sum()==5930 and not np.any(remaining&~deferred)
 labels,n=ndimage.label(remaining,np.ones((3,3))); assert n==303
 source=np.array(Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA'))
 overlaps={}; union=np.zeros_like(remaining)
 for item in read(OUT/'review-mask-inventory.json')['masks']:
  x,y=item['box_top_left']; a=mask(Path(item['png'])); h,w=a.shape
  full=np.zeros_like(remaining); x0,y0=max(0,x),max(0,y); x1,y1=min(full.shape[1],x+w),min(full.shape[0],y+h)
  full[y0:y1,x0:x1]=a[y0-y:y1-y,x0-x:x1-x]
  hit=full&remaining
  if hit.any(): overlaps[item['index']]=hit; union|=hit
 unassigned=remaining&~union; assert unassigned.sum()==26
 # Only small coordinate sets are sampled against phase alpha; masks never acquire ownership here.
 yy,xx=np.where(unassigned); phase=[[] for _ in xx]
 for f in read(OUT/'ground-texture-preparation/state-source-preservation.json')['frames']:
  x,y,w,h=f['bbox']; inside=(xx>=x)&(xx<x+w)&(yy>=y)&(yy<y+h)
  if not inside.any(): continue
  alpha=np.array(Image.open(f['image']).convert('RGBA'))[:,:,3]
  for k in np.flatnonzero(inside):
   if alpha[yy[k]-y,xx[k]-x]: phase[k].append({q:f[q] for q in ('patch','name','state','frame','sha256')})
 groups=defaultdict(lambda:dict(pixels=0,components=[])); components=[]
 for i in range(1,n+1):
  hit=labels==i; ys,xs=np.where(hit)
  counts=sorted([dict(index=k,pixels=int((hit&m).sum())) for k,m in overlaps.items() if (hit&m).any()],key=lambda v:-v['pixels'])
  row=dict(component=i,pixels=int(hit.sum()),bounds=[int(xs.min()),int(ys.min()),int(xs.max()+1),int(ys.max()+1)],source_context_overlaps=counts)
  components.append(row); key=str(counts[0]['index']) if counts else 'no-static-mask'
  groups[key]['pixels']+=row['pixels']; groups[key]['components'].append(i)
 Image.fromarray(remaining.astype('uint8')*255).save(D/'remaining5930.png')
 Image.fromarray(unassigned.astype('uint8')*255).save(D/'no-static-mask26.png')
 overlay=source.copy(); overlay[remaining,:3]=[255,155,30]; overlay[unassigned,:3]=[255,40,120]
 Image.fromarray(overlay).save(D/'whole-map-residual-overlay.png')
 largest=sorted(components,key=lambda c:-c['pixels'])[:20]
 sheet=Image.new('RGB',(1200,1200),'#292929'); draw=ImageDraw.Draw(sheet)
 for k,c in enumerate(largest):
  x0,y0,x1,y1=c['bounds']; box=(max(0,x0-15),max(0,y0-15),min(source.shape[1],x1+15),min(source.shape[0],y1+15))
  pic=Image.fromarray(overlay).crop(box).convert('RGB'); scale=min(230/pic.width,245/pic.height); pic=pic.resize((round(pic.width*scale),round(pic.height*scale)),Image.Resampling.NEAREST)
  x=k%5*240;y=k//5*300;sheet.paste(pic,(x,y+45));draw.text((x+3,y+5),f"Component {c['component']}: {c['pixels']}px",fill='white');draw.text((x+3,y+22),'Masks '+','.join(str(v['index']) for v in c['source_context_overlaps'][:3]),fill='white')
 sheet.save(D/'largest20-source-contexts.png')
 report=dict(status='Read-only exhaustive atlas inventory; no fill or native-return authorization',model_sha256=sha(base/'model.blend'),atlas_sha256=sha(base/'composite.png'),remaining_nonrelief_pixels=int(remaining.sum()),component_count=n,all_remaining_inside_original_state_deferred_floor=True,static_source_mask_overlap_pixels=int(union.sum()),no_static_mask_pixels=26,separate_relief_pixels=int(relief.sum()),gray_inside_separate_relief=int((gray&relief).sum()),gray_known_pixels=int((gray&known).sum()),groups=dict(groups),components=components,no_static_mask_phase_records=[dict(x=int(x),y=int(y),frames=f) for x,y,f in zip(xx,yy,phase)],limitations=['Global atlas census, independent of inspected camera crops; it captures every exact neutral-gray non-relief floor texel in this saved candidate.','Source overlap is context, not exclusive ownership or proof of foreground geometry coverage. All source artwork stays on its existing receiver.','The 26 pixels outside static masks include initial and transition patch alpha; none is automatically a native return.','Separate relief remains a distinct bank/rock first-hit domain and requires its own physical coverage audit; filling it as generic floor is not authorized.','No geometry, atlas, state overlay, catalog, API call or bake changed.'],inputs={str(p.relative_to(OUT)):sha(p) for p in [base/'composite.png',base/'known-native-domain.png',prep/'separate_relief.png',prep/'deferred_state_floor.png',OUT/'review-mask-inventory.json',OUT/'ground-texture-preparation/state-source-preservation.json']})
 (D/'inventory.json').write_text(json.dumps(report,indent=2)+'\n')
 print(json.dumps({k:report[k] for k in ['remaining_nonrelief_pixels','component_count','static_source_mask_overlap_pixels','no_static_mask_pixels','separate_relief_pixels','gray_inside_separate_relief','gray_known_pixels','groups']},indent=2))
if __name__=='__main__': main()
