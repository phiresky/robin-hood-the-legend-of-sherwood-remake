"""Prepare finite legacy bank-underlay reuse scope without repainting bank surfaces."""
import json,hashlib,shutil,html
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
from scipy import ndimage
ROOT=Path(__file__).resolve().parents[3];OUT=ROOT/'level-editor/work/croisement02-refinement';D=OUT/'restart4-bank-underlay-input-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def read(p):return json.loads(p.read_text())
def mask(p):return np.array(Image.open(p).convert('L'))>0
def write(p,v):p.write_text(json.dumps(v,indent=2)+'\n')
def main():
 assert shutil.disk_usage(OUT).free>23*2**30;D.mkdir(exist_ok=False)
 b=OUT/'restart4-remaining-floor-bake-v1';known=mask(b/'known-native-domain.png');domain=mask(OUT/'restart2-ground-completion/preparation-v1/separate_relief.png');other=mask(OUT/'restart4-floor-closure-input-v1/domain.png');base=np.array(Image.open(b/'composite.png').convert('RGBA'));source=np.array(Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA'));rawpath=OUT/'restart2-ground-completion/approved-fill-retry-v2/generation-short-no-mask-with-lighting-openrouter-with-auxiliary/generated-raw.png';raw=np.array(Image.open(rawpath).convert('RGBA'))
 assert domain.sum()==569880 and not(domain&(known|other)).any() and np.all(base[domain,:3]==127)
 proposed=base.copy();proposed[domain,:3]=raw[domain,:3]
 Image.fromarray(proposed).save(D/'proposed-appearance.png');Image.fromarray(domain.astype('uint8')*255).save(D/'inferred-domain.png');guide=source.copy();guide[domain,:3]=(guide[domain,:3]*.3+np.array([230,150,35])*.7).astype('uint8');Image.fromarray(guide).save(D/'source-region-guide.png')
 labels,n=ndimage.label(domain,np.ones((3,3)));comps=[]
 for i in range(1,n+1):
  yy,xx=np.where(labels==i);comps.append(dict(pixels=len(xx),bounds=[int(xx.min()),int(yy.min()),int(xx.max()+1),int(yy.max()+1)]))
 regions=[]
 for y0 in range(0,1152,288):
  for x0 in range(0,1792,448):
   if domain[y0:y0+288,x0:x0+448].any():regions.append((x0,y0,min(1792,x0+448),min(1152,y0+288)))
 for page in range((len(regions)+3)//4):
  sheet=Image.new('RGB',(1200,4*240),'#292929');draw=ImageDraw.Draw(sheet)
  for row,box in enumerate(regions[page*4:page*4+4]):
   x0,y0,x1,y1=box;draw.text((5,row*240+4),f'Native map region {box}: {int(domain[y0:y1,x0:x1].sum())} inferred underlay pixels',fill='white')
   for col,(a,label) in enumerate([(source,'Original source'),(guide,'Orange: legacy underlay'),(base,'Approved flat-ground atlas'),(proposed,'Proposed inferred flat floor')]):
    pic=Image.fromarray(a).crop(box).convert('RGB');pic.thumbnail((295,205));sheet.paste(pic,(col*300,row*240+32));draw.text((col*300+4,row*240+17),label,fill='white')
  sheet.save(D/f'full-domain-regions-{page}.png')
 orbit=OUT/'restart2-textures/batch10-linked-static-review-v1/orbit-512';context=Image.new('RGB',(1200,420),'#292929');draw=ImageDraw.Draw(context)
 for col,(view,box,label) in enumerate([(0,(0,0,1024,768),'Original game camera current scene'),(4,(315,550,375,600),'Reverse gray patch: ground first hit'),(2,(320,540,355,580),'Other side: already pending5930')]):
  im=Image.open(orbit/f'view-{view}.png').crop(box);im.thumbnail((395,380));
  if view:im=im.resize((round(im.width*min(395/im.width,380/im.height)),round(im.height*min(395/im.width,380/im.height))),Image.Resampling.NEAREST)
  context.paste(im,(col*400,32));draw.text((col*400+4,5),label,fill='white')
 context.save(D/'physical-context.png')
 native=read(OUT/'ground-receiver-review-v5/reference/packet.json');physical=read(OUT/'restart2-textures/batch10-oblique-patch-attribution-v1/report.json');hits=[]
 for s in physical['samples']:
  if s.get('asset_group')=='croisement02-ground-receiver':
   x,y=s['texture_samples'][0]['atlas_pixel_top_left'];hits.append(dict(view=s['view'],pixel=s['pixel'],atlas=[x,y],legacy_domain=bool(domain[y,x]),pending5930=bool(other[y,x])))
 summary=dict(status='Private whole legacy flat-floor underlay input proposal; no permission or bake',base_model=str(b/'model.blend'),base_model_sha256=sha(b/'model.blend'),inferred_pixels=int(domain.sum()),native_returns=0,known_preserved=int(known.sum()),outside_preserved=int((~domain).sum()),raw_response=str(rawpath),raw_sha256=sha(rawpath),components=comps,legacy_source=dict(bank_model_sha256=native['bank_model_sha256'],bank_first_hit_sha256=native['bank_first_hit_sha256'],interpretation='Complement of native ground first-hit using old bank5fef plus flat ground. Not a current arbitrary-angle bank coverage proof.'),current_oblique_scene_sha256=physical['scene_sha256'],physical_report_sha256=sha(OUT/'restart2-textures/batch10-oblique-patch-attribution-v1/report.json'),ground_samples=hits,disjoint_from_pending5930=True,bank_model_or_material_change=False,ground_geometry_change=False,source_ownership_transfer=False,refs=read(OUT/'restart2-ground-completion/preparation-v1/inventory.json')['supplementary_references'],limitations=['Only84 sampled reverse-view centers prove exposed legacy-domain floor; all other pixels are proposed conservative unseen underlay, not claimed visible.','Ground is already an approved continuous flat plane. The proposal gives unknown portions inferred floor appearance beneath bank; bank surface artwork/height/geometry stays untouched.','Current bank relief or source geometry gaps remain independent; filling the plane cannot fix bank geometry.','Existing raw floor includes soft texture and inferred shadows. No new synthesis is requested.','Pending5930 is disjoint. Future writer must compose both approved deltas onto latest approved ground, not roll back either.','If this entire scope is approved and baked, no neutral-gray flat-ground atlas pixels remain; that does not establish complete asset geometry/material coverage.'],files={p.name:sha(p) for p in D.glob('*.png')})
 write(D/'proposal.json',summary);write(D/'validation.json',dict(status='PASS',proposal_sha256=sha(D/'proposal.json'),known_exact=bool(np.array_equal(proposed[known],base[known])),outside_exact=bool(np.array_equal(proposed[~domain],base[~domain])),alpha_exact=bool(np.array_equal(proposed[:,:,3],base[:,:,3])),remaining_gray_outside_domain=int((np.all(proposed[:,:,:3]==127,axis=2)).sum()),disjoint_pending5930=True,model_writes=0,api_calls=0))
 images=[('physical-context.png','Current physical context: original camera first'),('source-region-guide.png','Whole finite legacy underlay domain'),*[(p.name,p.stem) for p in sorted(D.glob('full-domain-regions-*.png'))]]
 sections=''.join(f'<h2>{label}</h2><a href="{name}"><img src="{name}"></a>' for name,label in images)
 (D/'index.html').write_text('<!doctype html><meta charset="utf-8"><title>Legacy bank underlay scope</title><style>body{background:#181b20;color:#eee;font:16px system-ui;max-width:1250px;margin:25px auto}img{max-width:100%}a{color:#acf}</style><h1>Legacy bank underlay — private input proposal</h1><p>569880 inferred flat-floor pixels, zero native returns. Bank geometry/material untouched. Only84 reverse-view samples establish exposure; remaining underlay is conservative inference. No synthesis or model bake.</p><p><a href="proposal.json">Exact scope/provenance</a> · <a href="validation.json">Guards</a></p>'+sections)
 print('proposal',sha(D/'proposal.json'),'components',n,'regions',len(regions),'MiB',sum(p.stat().st_size for p in D.iterdir())/2**20)
if __name__=='__main__':main()
