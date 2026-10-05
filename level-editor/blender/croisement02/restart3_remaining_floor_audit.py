"""Map fixed context-view gray diagnostics back to bounded flat-ground atlas regions."""
import json,hashlib,math
from pathlib import Path
from collections import Counter
import numpy as np
from PIL import Image,ImageDraw
from scipy import ndimage
ROOT=Path(__file__).resolve().parents[3];OUT=ROOT/'level-editor/work/croisement02-refinement';D=OUT/'restart3-remaining-floor-audit-v1';CTX=OUT/'restart2-state/underlay-context-proposal-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def mask(p):return np.array(Image.open(p).convert('L'))>0
def write(p,v):p.write_text(json.dumps(v,indent=2)+'\n')
def main():
 D.mkdir(exist_ok=False);manifest=json.loads((CTX/'manifest.json').read_text());basepath=OUT/'restart2-state/underlay-input-review-v1/proposed-appearance.png';base=np.array(Image.open(basepath).convert('RGBA'));shape=base.shape[:2]
 known=mask(OUT/'restart2-ground-completion/preparation-v1/known.png');relief=mask(OUT/'restart2-ground-completion/preparation-v1/separate_relief.png');fence=mask(OUT/'restart3-initial-fence/floor-proposal-v2/inferred-hidden-floor.png');tree45=mask(OUT/'restart3-initial-fence/tree45-floor-proposal-v1/inferred-floor-domain.png');state=mask(OUT/'restart2-state/underlay-aggregate-audit-v3/combined-candidate-domain.png')
 gray=np.all(base[:,:,:3]==127,axis=2);remaining=gray&~(known|relief|fence|tree45|state)
 labels,n=ndimage.label(remaining,np.ones((3,3)));slices=ndimage.find_objects(labels);union=np.zeros(shape,bool);footprints={};records=[]
 groups={'log-trap':['croisement02-log-trap-applied'],'south-cart':['croisement02-south-cart-terminal-wreck-body','croisement02-south-cart-terminal-cask','croisement02-south-cart-terminal-loose-wood'],'north-cart':['croisement02-north-cart-terminal-physical']}
 endpoints={r['id']:r for r in manifest['endpoints']};sin=math.sin(math.radians(35))
 for family,ids in groups.items():
  parts=[p for key in ids for p in endpoints[key]['parts']];lo=np.min([p['bounds'][0]for p in parts],0);hi=np.max([p['bounds'][1]for p in parts],0);center=(lo+hi)/2;scale=max(240,np.linalg.norm(hi-lo)*1.35)
  for view in ['native','oblique']:
   r=next(r for r in manifest['records']if r['family']==family and r['view']==view and r['appearance']=='proposal');p=CTX/r['file'];assert sha(p)==r['sha256'];im=np.array(Image.open(p).convert('RGBA'));d=np.array(r['direction']);d/=np.linalg.norm(d);right=np.cross([0,0,1],d);right/=np.linalg.norm(right);up=np.cross(d,right)
   yy,xx=np.indices(im.shape[:2]);sx=((xx+.5)/640-.5)*scale;sy=(.5-(yy+.5)/640)*scale;world=center+sx[:,:,None]*right+sy[:,:,None]*up;world-=world[:,:,2:3]/d[2]*d
   gx=np.floor(world[:,:,0]).astype(int);gy=np.floor(-world[:,:,1]*sin).astype(int);valid=(gx>=0)&(gx<shape[1])&(gy>=0)&(gy<shape[0]);cx=np.clip(gx,0,shape[1]-1);cy=np.clip(gy,0,shape[0]-1)
   neutral=(im[:,:,:3].max(2)-im[:,:,:3].min(2)<=1)&(im[:,:,:3].mean(2)>=125)&(im[:,:,:3].mean(2)<=129)&(im[:,:,3]>250)&valid
   projected=np.zeros(shape,bool);projected[cy[neutral],cx[neutral]]=True;footprints[family+'-'+view]=projected;union|=projected
   roles={'prior-fence5419':fence,'prior-tree45-2441':tree45,'state8201':state,'known-original':known,'separate-relief':relief,'new-gray-floor':remaining,'atlas-not-neutral':~gray}
   counts={k:int((projected&m).sum())for k,m in roles.items()};touched=Counter(labels[projected&remaining].tolist())
   overlay=im.copy();colors={'prior-fence5419':(40,170,240),'prior-tree45-2441':(120,100,240),'state8201':(50,210,140),'known-original':(240,50,50),'separate-relief':(210,50,190),'new-gray-floor':(240,150,40),'atlas-not-neutral':(240,50,50)}
   for key,m in roles.items():sel=neutral&m[cy,cx];overlay[sel,:3]=colors[key]
   Image.fromarray(overlay).save(D/(family+'-'+view+'-classified.png'))
   records.append(dict(family=family,view=view,image_sha256=sha(p),center=center.tolist(),ortho_scale=scale,direction=d.tolist(),neutral_render_pixels=int(neutral.sum()),unique_projected_atlas_pixels=int(projected.sum()),counts=counts,touched_components=dict(touched)))
 source=np.array(Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA'));inventory=json.loads((OUT/'review-mask-inventory.json').read_text())['masks'];components=[];candidate=np.zeros(shape,bool)
 for label,count in Counter(labels[union&remaining].tolist()).most_common():
  sl=slices[label-1];ys,xs=sl;component=labels==label;area=int(component.sum());bounds=[xs.start,ys.start,xs.stop,ys.stop];overlaps=[]
  for row in inventory:
   x,y=row['box_top_left'];w,h=row['box_size'];x0=max(x,xs.start);x1=min(x+w,xs.stop);y0=max(y,ys.start);y1=min(y+h,ys.stop)
   if x1<=x0 or y1<=y0:continue
   a=mask(Path(row['png']));amount=int((component[y0:y1,x0:x1]&a[y0-y:y1-y,x0-x:x1-x]).sum())
   if amount:overlaps.append(dict(index=row['index'],pixels=amount,obstacles=row.get('obstacle_indices',[])))
  overlaps.sort(key=lambda r:-r['pixels']);components.append(dict(component=label,atlas_pixels=area,visible_diagnostic_pixels=count,bounds=bounds,source_masks=overlaps,families=[r['family']+'-'+r['view']for r in records if label in r['touched_components']]))
  candidate|=component
  box=(max(0,xs.start-12),max(0,ys.start-12),min(shape[1],xs.stop+12),min(shape[0],ys.stop+12));over=source.copy();over[component,:3]=(over[component,:3]*.35+np.array([235,140,35])*.65).astype('uint8');im=Image.new('RGB',((box[2]-box[0])*2,box[3]-box[1]));im.paste(Image.fromarray(source).convert('RGB').crop(box),(0,0));im.paste(Image.fromarray(over).convert('RGB').crop(box),(box[2]-box[0],0));im.save(D/f'component-{label}-source.png')
 Image.fromarray(candidate.astype('uint8')*255).save(D/'connected-gray-candidate-union.png');Image.fromarray((union&remaining).astype('uint8')*255).save(D/'visible-gray-projection.png')
 write(D/'report.json',dict(status='Read-only diagnostics, not an approved domain or physical first-hit proof',context_manifest_sha256=sha(CTX/'manifest.json'),atlas_sha256=sha(basepath),method='Reconstruct exact orthographic camera from frozen endpoint bounds/directions and intersect rendered neutral pixels with Z0 plane. Only matching gray atlas texels outside protected/prior domains seed connected gray components.',records=records,components=components,candidate_union_pixels=int(candidate.sum()),candidate_union_sha256=sha(D/'connected-gray-candidate-union.png'),known_overlap=int((candidate&known).sum()),relief_overlap=int((candidate&relief).sum()),pending_overlap=int((candidate&(fence|tree45|state)).sum()),limitations=['Gray RGB is only a diagnostic; flat-plane projection is not first-hit receiver proof.','Bank or object material can project to unrelated floor coordinates; source components and multi-view overlap require review before proposal.','Connected-component expansion is a bounded diagnostic completion set, not permission to fill.','No model, image atlas, source ownership, API or renderer changed.']))
 print(json.dumps({'records':[(r['family'],r['view'],r['counts'])for r in records],'components':[{k:v for k,v in r.items()if k!='source_masks'}for r in components],'candidate_pixels':int(candidate.sum())},indent=2))
if __name__=='__main__':main()
