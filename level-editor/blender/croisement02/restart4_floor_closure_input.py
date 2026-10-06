"""Prepare finite remaining floor inputs, preserving foreground and phase ownership."""
import json,hashlib,shutil
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];OUT=ROOT/'level-editor/work/croisement02-refinement';D=OUT/'restart4-floor-closure-input-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def read(p):return json.loads(p.read_text())
def write(p,v):p.write_text(json.dumps(v,indent=2)+'\n')
def mask(p):return np.array(Image.open(p).convert('L'))>0
def main():
 assert shutil.disk_usage(OUT).free>23*2**30
 D.mkdir(exist_ok=False)
 prior=OUT/'restart4-remaining-floor-bake-v1';inv=OUT/'restart4-ground-closure-inventory-v1';report=read(inv/'inventory.json');domain=mask(inv/'remaining5930.png');known=mask(prior/'known-native-domain.png');relief=mask(OUT/'restart2-ground-completion/preparation-v1/separate_relief.png')
 base=np.array(Image.open(prior/'composite.png').convert('RGBA'));source=np.array(Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA'));background=np.array(Image.open(OUT/'source-states/covered.png').convert('RGBA'))
 rawpath=OUT/'restart2-ground-completion/approved-fill-retry-v2/generation-short-no-mask-with-lighting-openrouter-with-auxiliary/generated-raw.png';raw=np.array(Image.open(rawpath).convert('RGBA'))
 native=np.zeros_like(domain);phase=[];frames=read(OUT/'ground-texture-preparation/state-source-preservation.json')['frames'];terminal={}
 for f in frames:
  if f['state']=='transition' and (f['patch'] not in terminal or terminal[f['patch']]['frame']<f['frame']):terminal[f['patch']]=f
 animations=read(OUT/'animation-references/manifest.json')['animations']
 for item in report['no_static_mask_phase_records']:
  x,y=item['x'],item['y'];ini=any(f['state']=='initial' for f in item['frames']);ends=[];ambient=[]
  for key in {f['patch'] for f in item['frames']}:
   f=terminal.get(key)
   if f:
    xx,yy,w,h=f['bbox']
    if xx<=x<xx+w and yy<=y<yy+h and np.array(Image.open(f['image']).convert('RGBA'))[y-yy,x-xx,3]:ends.append(key)
  for a in animations:
   f=a['frames'][0];xx,yy,w,h=f['bbox']
   if xx<=x<xx+w and yy<=y<yy+h and np.array(Image.open(f['image']).convert('RGBA'))[y-yy,x-xx,3]:ambient.append(a['index'])
  native[y,x]=not ini and not ends and not ambient and np.array_equal(source[y,x],background[y,x])
  phase.append(dict(x=x,y=y,initial=ini,terminal=ends,ambient_frame0=ambient,native_return=bool(native[y,x]),source_equal=bool(np.array_equal(source[y,x],background[y,x]))))
 inferred=domain&~native; proposed=base.copy();proposed[inferred,:3]=raw[inferred,:3];proposed[native]=background[native]
 physical=read(OUT/'restart2-textures/batch10-residual-floor-attribution-v1/report.json');exposed=np.zeros_like(domain)
 for s in physical['samples']:
  if s['asset_group']=='croisement02-ground-receiver':x,y=s['pixel'];exposed[y,x]=True
 assert exposed.sum()==487 and not np.any(exposed&~domain)
 assert not np.any(domain&(known|relief)) and np.array_equal(proposed[~domain],base[~domain]) and np.array_equal(proposed[:,:,3],base[:,:,3])
 for name,a in [('domain',domain),('inferred-domain',inferred),('native-return-domain',native),('native-exposed487',exposed),('native-covered5443',domain&~exposed)]:Image.fromarray(a.astype('uint8')*255).save(D/(name+'.png'))
 Image.fromarray(proposed).save(D/'proposed-appearance.png');guide=base.copy();guide[inferred,:3]=[245,155,35];guide[native,:3]=[20,225,235];Image.fromarray(guide).save(D/'region-guide.png')
 # Every component appears in the exact mask; grouped closeups cover all source contexts.
 groups=sorted(report['groups'].items(),key=lambda kv:-kv[1]['pixels']);comp={c['component']:c for c in report['components']}
 for page in range((len(groups)+7)//8):
  sheet=Image.new('RGB',(1200,8*230),'#292929');draw=ImageDraw.Draw(sheet)
  for row,(key,g) in enumerate(groups[page*8:page*8+8]):
   cs=[comp[i] for i in g['components']];largest=max(cs,key=lambda c:c['pixels']);x0,y0,x1,y1=largest['bounds'];box=(max(0,x0-12),max(0,y0-12),min(1792,x1+12),min(1152,y1+12));draw.text((4,row*230+3),f"Source context {key}: {g['pixels']}px / {len(cs)} components; largest shown",fill='white')
   overlay=source.copy();overlay[domain,:3]=[240,155,35];overlay[exposed,:3]=[255,50,100]
   for col,(a,label) in enumerate([(source,'Original art'),(overlay,'Orange covered; pink exposed'),(base,'Approved ground atlas'),(proposed,'Proposed underlying floor')]):
    pic=Image.fromarray(a).crop(box).convert('RGB');scale=min(292/pic.width,190/pic.height);pic=pic.resize((round(pic.width*scale),round(pic.height*scale)),Image.Resampling.NEAREST);sheet.paste(pic,(col*300,row*230+38));draw.text((col*300+4,row*230+20),label,fill='white')
  sheet.save(D/f'source-reuse-contexts-{page}.png')
 summary=dict(status='Private input proposal; root and user scope approval pending',base_model=str(prior/'model.blend'),base_model_sha256=sha(prior/'model.blend'),base_atlas_sha256=sha(prior/'composite.png'),raw_response=str(rawpath),raw_sha256=sha(rawpath),domain_pixels=int(domain.sum()),inferred_pixels=int(inferred.sum()),native_return_pixels=int(native.sum()),known_preserved=int(known.sum()),outside_preserved=int((~domain).sum()),source_ownership_transfer=False,geometry_change=False,api_requested=False,physical_native_visibility=dict(scene_sha256=physical['scene_sha256'],report_sha256=sha(OUT/'restart2-textures/batch10-residual-floor-attribution-v1/report.json'),exposed=487,covered=5443,no_hit=0),phase_pixels=phase,refs=read(OUT/'restart2-ground-completion/preparation-v1/inventory.json')['supplementary_references'],limitations=['487 exposed native centers include source wood and edge mismatches; inferred underlying floor does not resolve missing foreground geometry.','5443 covered pixels are hidden from original camera; fill is conservative underlying-floor completion for other views, not source reassignment.','Elevated hiding-Pc initial and terminal artwork stays on independent state overlays; no foreground art is copied onto ground.','Pixel191,548 belongs to animated Arbre03 frame0; its static floor receives inferred texture only.','Legacy569880 bank-underlay pixels remain excluded; oblique gray triangle attribution is a separate pending physical audit.','Only exact displayed reuse/native domain is proposed. Saved-model bake and appearance review follow scope approval.'],files={p.name:sha(p) for p in D.glob('*.png')})
 write(D/'proposal.json',summary);write(D/'validation.json',dict(status='PASS',proposal_sha256=sha(D/'proposal.json'),known_exact=True,outside_exact=True,alpha_exact=True,geometry_untouched=True,output_bytes=sum(p.stat().st_size for p in D.iterdir()),synthesis_calls=0));print(json.dumps({k:summary[k] for k in ['domain_pixels','inferred_pixels','native_return_pixels','known_preserved']}))
 receipt=OUT/'restart3-review-batches/batch-v10/user-approval.json';assert sha(receipt)=='534d552590823cbb5221e1ff52a082657360eb16f80ff5c42acfdb4980a2e3e8';approval=prior/'user-appearance-approval.json'
 if not approval.exists():write(approval,dict(status='user-approved',scope='saved-model appearance only',model_sha256=summary['base_model_sha256'],receipt=str(receipt),receipt_sha256=sha(receipt),answer=read(receipt)['answer'],review_revision='b64d46f5b48dde562026ffabe1c6aef8cc9bbb7e944ac8ebba974b6027087dde'))
if __name__=='__main__':main()
