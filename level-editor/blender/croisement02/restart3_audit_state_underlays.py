from pathlib import Path
import json,hashlib
import numpy as np
from PIL import Image,ImageDraw
r=Path('level-editor/work/croisement02-refinement');out=r/'restart2-state/underlay-aggregate-audit-v3';prep=r/'restart2-ground-completion/preparation-v1';load=lambda p:np.array(Image.open(p).convert('L'))>0;deferred=load(prep/'deferred_state_floor.png');known=load(prep/'known.png');relief=load(prep/'separate_relief.png');fence=np.zeros(deferred.shape,bool);fence[811:963,1018:1170]=True;prior=np.zeros(deferred.shape,bool)
for x,y in [(329,366),(332,370),(334,372),(294,376),(337,376),(292,377),(290,378),(286,380),(284,381),(282,382),(280,383),(276,385),(274,386),(272,387)]:prior[y,x]=True
levelpath=Path('level-editor/library/game-data/Data/Levels/Croisement02.rhp.json');level=json.loads(levelpath.read_text());
base=np.array(Image.open(r/'restart3-fence-receiver/terminal-v3/base-atlas.png').convert('RGBA'));rawpath=r/'restart2-ground-completion/approved-fill-retry-v2/generation-short-no-mask-with-lighting-openrouter-with-auxiliary/generated-raw.png';raw=np.array(Image.open(rawpath).convert('RGBA'));records=[];combined=np.zeros(deferred.shape,bool);sheet=Image.new('RGB',(1200,1400),'#242424');draw=ImageDraw.Draw(sheet)
def add(canvas,path,x,y,hashvalue=None):
 b=path.read_bytes()
 if hashvalue:assert hashlib.sha256(b).hexdigest()==hashvalue,path
 a=np.array(Image.open(path).convert('RGBA'))[:,:,3]>0;h,w=a.shape;x=int(x);y=int(y);l=max(0,x);t=max(0,y);rr=min(canvas.shape[1],x+w);bb=min(canvas.shape[0],y+h)
 if l<rr and t<bb:canvas[t:bb,l:rr]|=a[t-y:bb-y,l-x:rr-x]
for i,name in enumerate(['log-trap','rock-trap','south-cart','north-cart']):
 sourcepath=r/'state-target-evidence'/name/'manifest.json';m=json.loads(sourcepath.read_text());union=np.zeros(deferred.shape,bool);frames=0
 for part in m['parts']:
  for f in ([part['initial']]if part.get('initial')else[])+part['frames']:
   add(union,Path(f['image']),part['position'][0]+f['offset'][0],part['position'][1]+f['offset'][1],f['image_sha256']);frames+=1
 for binding in m['background_bindings']:
  for state in binding['states'].values():
   for f in state.get('frames',[]):add(union,r/'source-states'/f['image'],*f['bbox'][:2]);frames+=1
 patch=level['patches'][m['native_metadata_patch']];native_masks=[]
 for ref in patch['old_masks']+patch['new_masks']:
  group=[(idx,row)for idx,row in enumerate(level['masks'])if row['layer']==ref['layer']];global_index,mask=group[ref['index']];path=r/'baseline/masks'/f'{global_index:06d}.png';a=np.array(Image.open(path).convert('L'))>0;x,y=mask['box_top_left'];h,w=a.shape;assert [w,h]==mask['box_size'];union[y:y+h,x:x+w]|=a;native_masks.append({'layer':ref['layer'],'layer_index':ref['index'],'global_index':global_index,'sha256':hashlib.sha256(path.read_bytes()).hexdigest()})
 candidate=deferred&union&~fence&~prior;assert not(candidate&(known|relief)).any();combined|=candidate
 Image.fromarray(union.astype('uint8')*255).save(out/(name+'-source-alpha-union.png'));Image.fromarray(candidate.astype('uint8')*255).save(out/(name+'-candidate-domain.png'))
 ys,xs=np.where(union);box=[max(0,int(xs.min())-5),max(0,int(ys.min())-5),min(1792,int(xs.max())+6),min(1152,int(ys.max())+6)];overlay=base.copy();overlay[candidate,:3]=(255,40,180);proposed=base.copy();proposed[candidate,:3]=raw[candidate,:3]
 records.append({'family':name,'native_state_masks':native_masks,'source_manifest_sha256':hashlib.sha256(sourcepath.read_bytes()).hexdigest(),'source_frames_verified':frames,'candidate_pixels':int(candidate.sum()),'source_alpha_union_pixels':int(union.sum()),'fence_pixels_excluded':int((deferred&union&fence).sum()),'already_approved14_excluded':int((deferred&union&prior).sum()),'raw_candidate_changes':int((candidate&np.any(base!=raw,axis=2)).sum()),'scope':'Existing ground-owned deferred floor under exact family artwork union; no new visible-object pixel ownership assigned.'})
 for j,(label,img)in enumerate([('approved floor',base),('pink proposed domain',overlay),('raw floor proposal',proposed)]):
  im=Image.fromarray(img).convert('RGB').crop(box);im.thumbnail((390,310));sheet.paste(im,(j*400,i*350+30));draw.text((j*400+5,i*350+5),name+' '+label,fill='white')
Image.fromarray(combined.astype('uint8')*255).save(out/'combined-candidate-domain.png');sheet.save(out/'source-domain-comparison.png');report={'status':'Private bounded underlay domain proposal; no API/material/selection change','records':records,'combined_candidate_pixels':int(combined.sum()),'level_source_sha256':hashlib.sha256(levelpath.read_bytes()).hexdigest(),'guards':{'known_source_pixels_edited':0,'separate_relief_pixels_edited':0,'initial_fence_domain_overlap':0,'approved14pixel_overlap':0},'raw_floor_sha256':hashlib.sha256(rawpath.read_bytes()).hexdigest(),'remaining':'Review actual compatible physical state context and exact raw appearance, then grouped appearance approval. Native dynamic patches stay unchanged, overlaid independently. Other gaps outside verified family union remain separate.'};(out/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2))
