"""Compare independently identified source domains and measure native FX correspondence."""
import hashlib
import json
from pathlib import Path
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];r=ROOT/'level-editor/work/active/croisement01';base=r/'restart2'
out=base/'tree14-static-crown-material-partition-v1';out.mkdir(exist_ok=False)
sha=lambda p:hashlib.sha256(Path(p).read_bytes()).hexdigest()
meta=json.loads((r/'baseline/masks/manifest.json').read_text());meta={m['index']:m for m in meta['masks']}
frames=json.loads((base/'tree14-native-crown-animation-source-v1/source.json').read_text())['frames']
# A small padded global-source window includes all fourteen FX frames and the
# entire native94 support. No checkerboard parity or color threshold defines a material.
left,top,right,bottom=640,35,990,350;width,height=right-left,bottom-top
pins={};domains={}
def domain(index,path=None):
 m=meta[index];p=Path(path) if path else r/'baseline/masks'/m['png'];pins[str(p.relative_to(ROOT))]=sha(p)
 a=np.asarray(Image.open(p).convert('L'))>0;x,y=m['box_top_left'];h,w=a.shape;result=np.zeros((height,width),bool)
 lx,ty,rx,by=max(left,x),max(top,y),min(right,x+w),min(bottom,y+h)
 if rx>lx and by>ty:result[ty-top:by-top,lx-left:rx-left]=a[ty-y:by-y,lx-x:rx-x]
 return result
native=domain(94)
for i in (9,11,12,15,16):domains[f'wood-{i}']=domain(i)
domains['wood-14']=domain(14,base/'tree14-source-restoration-proposal-v1/wood-domain.png')
for i in (79,81,84,85,86):domains[f'plant-{i}']=domain(i)
wood=np.logical_or.reduce([a for k,a in domains.items() if k.startswith('wood')])
plants=np.logical_or.reduce([a for k,a in domains.items() if k.startswith('plant')])
edge=domain(14)&~domain(14,base/'tree14-source-restoration-proposal-v1/wood-domain.png')
conflict=wood&plants
candidate=native&~(wood|plants|edge)
labels=np.zeros((height,width),np.uint8);labels[native]=1;labels[native&wood]=2;labels[native&plants]=3;labels[native&edge]=4;labels[native&conflict]=5
source=Image.open(r/'baseline/covered.png').convert('RGBA').crop((left,top,right,bottom));source_arr=np.asarray(source).copy()
for name,a in [('native94',native),('wood-domain-intersection',native&wood),('plant-domain-intersection',native&plants),('unresolved-edge',native&edge),('material-conflict',native&conflict),('foliage-candidate',candidate)]:
 Image.fromarray(a.astype(np.uint8)*255).save(out/(name+'-domain.png'))
 rgba=source_arr.copy();rgba[:,:,3]=a*255;Image.fromarray(rgba).save(out/(name+'-source.png'))
Image.fromarray(labels).save(out/'static-material-labels.png')
arrays=[]
for f in frames:
 p=ROOT/f['path'];assert sha(p)==f['sha256'];raw=np.asarray(Image.open(p).convert('RGBA'));x,y,w,h=f['bbox'];a=np.zeros((height,width,4),np.uint8);a[y-top:y-top+h,x-left:x-left+w]=raw;arrays.append(a)
correspondence=[]
for number,a in enumerate(arrays):
 rows=[]
 for name,mask in domains.items():
  yy,xx=np.nonzero(mask&native&(arrays[0][:,:,3]>0))
  if len(xx)<16:continue
  ref=arrays[0][yy,xx,:3].astype(float);scores=[]
  for dy in range(-3,4):
   for dx in range(-3,4):
    valid=(xx+dx>=0)&(xx+dx<width)&(yy+dy>=0)&(yy+dy<height)
    covered=np.zeros(len(xx),bool);covered[valid]=a[yy[valid]+dy,xx[valid]+dx,3]>0
    n=int(covered.sum())
    if n<max(8,len(xx)//3):continue
    error=float(np.abs(a[yy[covered]+dy,xx[covered]+dx,:3].astype(float)-ref[covered]).mean())
    score=error+25*(1-n/len(xx));scores.append(dict(dx=dx,dy=dy,mean_absolute_rgb_error=error,matched_samples=n,reference_samples=len(xx),score=score))
  scores.sort(key=lambda x:x['score']);rows.append(dict(material=name,best_candidates=scores[:3],assignment_status='MEASURED_ONLY_NO_WARP_APPLIED'))
 active=a[:,:,3]>0
 correspondence.append(dict(frame=number,native_pixels=int(active.sum()),static_domain_counts={str(k):int(np.count_nonzero(active&(labels==k))) for k in range(6)},regions=rows))
report=dict(status='MATERIAL_PARTITION_PROPOSAL_REQUIRES_SOURCE_AND_GEOMETRY_REVIEW',bbox_source=[left,top,right,bottom],source_sha256=sha(r/'baseline/covered.png'),mask_pins=pins,
 counts={'native94':int(native.sum()),'wood':int((native&wood).sum()),'plants':int((native&plants).sum()),'unresolved_edge':int((native&edge).sum()),'wood_plant_conflict':int((native&conflict).sum()),'foliage_candidate':int(candidate.sum())},
 independent_material_evidence={'wood_masks':[9,11,12,15,16],'wood14_domain':'tree14-source-restoration-proposal-v1/wood-domain.png','plant_masks':[79,81,84,85,86]},
 source_rule='Independent woody/plant occupancy domains constrain possible ownership but are not final material masks. Foreground foliage can overlap a wood domain. FX correspondence is measured inside those domains; parity and color alone never assign a material.',
 limitations=['Wood and plant domain intersections contain foreground overlap and require a separate visual material trace.','Remaining foliage is a candidate, not a reviewed physical-opacity mask.','Overlapping crown93/96 ownership and hidden geometry are unresolved.','FX pixels beyond static support or ambiguous shifts need further correspondence review.','No source pixel is authorized for synthesis by this diagnostic.'])
(out/'partition.json').write_text(json.dumps(report,indent=2)+'\n');(out/'frame-correspondence.json').write_text(json.dumps(correspondence,indent=2)+'\n');print(report['counts'])
