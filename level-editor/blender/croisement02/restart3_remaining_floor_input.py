"""Freeze a bounded floor-reuse proposal and exact transient-reservation background return."""
import json,hashlib,shutil
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];OUT=ROOT/'level-editor/work/croisement02-refinement';D=OUT/'restart3-remaining-floor-input-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def rgba(p):return np.array(Image.open(p).convert('RGBA'))
def mask(p):return np.array(Image.open(p).convert('L'))>0
def write(p,v):p.write_text(json.dumps(v,indent=2)+'\n')
def main():
 assert shutil.disk_usage(OUT).free>25*1024**3;D.mkdir(exist_ok=False)
 audit=OUT/'restart3-remaining-floor-audit-v1';reuse=OUT/'restart3-remaining-floor-reuse-review-v1';model=OUT/'restart3-ground-reuse-combined-v1/model.blend';basepath=model.parent/'composite.png';base=rgba(basepath);sourcepath=OUT/'animation-references/composite-frame-0.png';source=rgba(sourcepath);coveredpath=OUT/'source-states/covered.png';covered=rgba(coveredpath);inferred=mask(audit/'proposed-inferred-floor-union.png');native=mask(audit/'unassigned49-hold.png');union=inferred|native
 assert inferred.sum()==14876 and native.sum()==49 and not(inferred&native).any() and np.array_equal(source[native],covered[native])
 layerspath=OUT/'source-states/layers.json';layers=json.loads(layerspath.read_text());patch=next(r for r in layers['mission_patches']if r['id']=='mission-Emb05_FoB_MP-patch-012');records=[];phaseunion=np.zeros_like(native)
 for state,info in patch['states'].items():
  for index,f in enumerate(info['frames']):
   p=OUT/'source-states'/f['image'];x,y,w,h=f['bbox'];a=rgba(p)[:,:,3]>0;overlap=np.zeros_like(native);overlap[y:y+h,x:x+w]=a;overlap&=native;count=int(overlap.sum())
   if count:phaseunion|=overlap
   records.append(dict(state=state,index=index,image=str(p),sha256=sha(p),overlap=count))
 assert phaseunion.sum()==49 and all(r['state']=='transition' and 1<=r['index']<=16 for r in records if r['overlap'])
 assert len(patch['states']['transition']['frames'])==30 and records[-1]['overlap']==0
 allframes=json.loads((OUT/'ground-texture-preparation/state-source-preservation.json').read_text())['frames'];otherhits=json.loads((audit/'held49-state-frames.json').read_text());assert len(otherhits)==16 and {r['patch']for r in otherhits}=={'mission-Emb05_FoB_MP-patch-012','mission-Tac19_FoB_EC-patch-008'} and all(1<=r['frame']<=8 for r in otherhits)
 replica=next(r for r in layers['mission_patches']if r['id']=='mission-Tac19_FoB_EC-patch-008');assert replica['state']['element_fx']['sprite']['position_x']==patch['state']['element_fx']['sprite']['position_x'] and replica['state']['element_fx']['sprite']['position_y']==patch['state']['element_fx']['sprite']['position_y']
 for state,info in replica['states'].items():
  assert len(info['frames'])==len(patch['states'][state]['frames'])
  for index,f in enumerate(info['frames']):
   pf=patch['states'][state]['frames'][index];assert f['bbox']==pf['bbox'] and np.array_equal(rgba(OUT/'source-states'/f['image']),rgba(OUT/'source-states'/pf['image']))
 known=mask(OUT/'restart2-ground-completion/preparation-v1/known.png');relief=mask(OUT/'restart2-ground-completion/preparation-v1/separate_relief.png');prior=mask(model.parent/'combined-domain.png')|mask(OUT/'restart3-initial-fence/floor-proposal-v2/inferred-hidden-floor.png');assert not(union&(known|relief|prior)).any()
 preview=rgba(reuse/'proposed-appearance.png');assert np.array_equal(preview[~inferred],base[~inferred]);preview[native]=source[native];assert np.array_equal(preview[~union],base[~union]) and np.array_equal(preview[:,:,3],base[:,:,3])
 for name,a in [('input.png',base),('proposed-appearance.png',preview)]:Image.fromarray(a).save(D/name)
 for name,a in [('inferred-domain.png',inferred),('native-return-domain.png',native),('combined-domain.png',union)]:Image.fromarray(a.astype('uint8')*255).save(D/name)
 editmask=np.full_like(base,255);editmask[union,3]=0;Image.fromarray(editmask).save(D/'mask.png');guide=base.copy();guide[inferred,:3]=[235,140,35];guide[native,:3]=[35,210,235];Image.fromarray(guide).save(D/'region-guide.png')
 box=(1353,230,1383,267);s=Image.new('RGB',(900,402),'#303030');dr=ImageDraw.Draw(s)
 for i,(label,a)in enumerate([('Current ground reservation',base),('Exact native initial background',source),('Proposed exact49 return',preview)]):s.paste(Image.fromarray(a).convert('RGB').crop(box).resize((300,370),Image.Resampling.NEAREST),(i*300,32));dr.text((i*300+3,8),label,fill='white')
 s.save(D/'native49-comparison.png')
 for name,ids in [('major-reuse-crops.png',['130','97','43','108','129','16','128']),('remaining-reuse-crops.png',['86','132','124','46','61','91','44'])]:
  pictures=[]
  for ident in ids:
   im=Image.open(reuse/f'source-{ident}-reuse.png').convert('RGB');w=1536;h=round(im.height*w/im.width);h=min(h,580);im.thumbnail((w,h));pictures.append((ident,im))
  sheet=Image.new('RGB',(1536,sum(im.height+28 for _,im in pictures)),'#303030');dr=ImageDraw.Draw(sheet);y=0
  for ident,im in pictures:dr.text((5,y+5),'Source context '+ident+' — native/source role / current floor / exact proposed reuse',fill='white');sheet.paste(im,(0,y+28));y+=im.height+28
  sheet.save(D/name)
 manifest=json.loads((reuse/'manifest.json').read_text());proposal=dict(status='INPUT PROPOSAL ONLY; no saved model or API change',base_model=str(model),base_model_sha256=sha(model),base_atlas_sha256=sha(basepath),base_appearance_status='868 combined saved-model appearance pending user approval; required dependency',inferred_pixels=14876,exact_native_returns=49,total_edited_pixels=14925,known772189_unchanged=True,all_outside_rgba_exact=int((~union).sum()),alpha_exact=True,bank_relief_unchanged=True,prior5419_2441_8201_14_unchanged=True,source_ownership_transfer=False,raw_response=manifest['raw_response'],raw_sha256=manifest['raw_sha256'],native_source_sha256=sha(sourcepath),covered_source_sha256=sha(coveredpath),native49_source_correspondence_exact=True,native49_patch_ids=[patch['id'],replica['id']],native49_initial_alpha_overlap=0,native49_terminal_alpha_overlap=0,native49_dynamic_transition_preserved=True,native49_semantics='Initial/reset restores the captured background; applied mode stamps final transition29. These49 coordinates occur only in temporary transition1..8 in both same-position mission replicas, so initial/terminal background stays exact native art.',proposal_only=True,approval_needed='Exact14876 inferred domain and shown raw reuse plus49 exact native ground returns. Local saved-model bake and appearance review follow separately. No new synthesis requested.',limitations=['Raw reused floor is softer/coarser than native pixels and conservatively continues nearby grass/soil/shadow.','Dominant source context labels do not transfer any foreground artwork ownership to ground.','This is the finite59-component completion set from six state-context views, not whole-map completion.'],images={p.name:sha(p)for p in D.glob('*.png')},source_frame_evidence=records,source_preservation_sha256=sha(OUT/'ground-texture-preparation/state-source-preservation.json'),layers_sha256=sha(layerspath),diagnostic_report_sha256=sha(audit/'completion-proposal.json'))
 write(D/'proposal.json',proposal);write(D/'validation.json',dict(status='PASS for input review',proposal_sha256=sha(D/'proposal.json'),inferred14876=True,native49_exact=True,all_outside_2049459_rgba_exact=True,known772189_exact=True,relief_and_prior_domains_exact=True,no_model_saved=True,no_api=True));print(D)
if __name__=='__main__':main()
