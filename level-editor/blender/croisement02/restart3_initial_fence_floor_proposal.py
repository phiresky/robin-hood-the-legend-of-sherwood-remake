"""Propose separately reviewed native-background returns and unseen initial floor."""
import json,hashlib
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];OUT=ROOT/'level-editor/work/croisement02-refinement'
D=OUT/'restart3-initial-fence/floor-proposal-v2'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 D.mkdir(exist_ok=False)
 fence=np.array(Image.open(OUT/'restart3-initial-fence/source-domain-v1/fence-source.png').convert('L'))>0
 # The prior eight-pixel return proposal was rejected after enlarged source review.
 polygons=[];returned=np.zeros_like(fence);inferred=fence.copy()
 srcpath=OUT/'animation-references/composite-frame-0.png';basepath=OUT/'restart3-fence-receiver/terminal-v3/base-atlas.png';source=np.array(Image.open(srcpath).convert('RGBA'));base=np.array(Image.open(basepath).convert('RGBA'));known=np.array(Image.open(OUT/'restart2-ground-completion/preparation-v1/known.png').convert('L'))>0
 assert not (fence&known).any()
 proposal=base.copy();proposal[returned]=source[returned];assert np.array_equal(proposal[~returned],base[~returned])
 for name,array in [('native-background-return',returned),('inferred-hidden-floor',inferred)]:Image.fromarray(array.astype('uint8')*255).save(D/(name+'.png'))
 Image.fromarray(proposal).save(D/'base-atlas-input-preview.png')
 overlay=source[:,:,:3].copy();overlay[returned]=[30,220,220];overlay[inferred]=(overlay[inferred].astype(float)*.35+np.array([215,110,25])*.65).astype('uint8')
 sheet=Image.new('RGB',(912,486),'#303030');draw=ImageDraw.Draw(sheet)
 for i,(title,arr)in enumerate([('Original source',source),('Orange: inferred floor; native fence artwork stays separate',overlay)]):sheet.paste(Image.fromarray(arr).convert('RGB').crop((1018,811,1170,963)).resize((456,456),Image.Resampling.NEAREST),(456*i,30));draw.text((456*i+5,8),title,fill='white')
 sheet.save(D/'source-role-proposal.png')
 report=dict(status='PROPOSAL ONLY: API off; source-domain and appearance review required',ground_model_sha256='16c638be71eeb76e86439a0fdb14bac1e7bb9562afe20d175b58d0df96fb4ec2',source_sha256=sha(srcpath),base_atlas_sha256=sha(basepath),source_polygons=polygons,native_background_return_pixels=int(returned.sum()),inferred_hidden_floor_pixels=int(inferred.sum()),scope='Only initial ground appearance beneath original fence domain. Fence source pixels remain on fence geometry; synthesized floor is explicitly unobserved and does not receive original wood RGB.',known_pixels_exact=int(known.sum()),neighbor_foliage_reserved=True,applied_terminal_unchanged=True,files={p.name:sha(p)for p in D.glob('*.png')},holds=['Complete bounded initial fence geometry review first.','No native ground reassignment: all5419 pixels are inferred underlying floor; original source wood remains on the fence.','Native source imagery and all previously known ground pixels remain unchanged; uncertain fence-edge source is never copied to ground.','No existing approval authorizes these newly editable floor texels.'])
 (D/'proposal.json').write_text(json.dumps(report,indent=2)+'\n');print(report['native_background_return_pixels'],report['inferred_hidden_floor_pixels'])
def prepare_inputs():
 d=D/'inputs-v1';d.mkdir(exist_ok=False)
 source=np.array(Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA'));input_image=Image.open(D/'base-atlas-input-preview.png').convert('RGBA');input_image.save(d/'input.png')
 editable=np.array(Image.open(D/'inferred-hidden-floor.png').convert('L'))>0
 guide=np.array(input_image);guide[editable,:3]=[235,140,35];Image.fromarray(guide).save(d/'region-guide.png')
 mask=np.full((1152,1792,4),255,dtype='uint8');mask[editable,3]=0;Image.fromarray(mask).save(d/'mask.png')
 known=np.array(Image.open(OUT/'restart2-ground-completion/preparation-v1/known.png').convert('L'))>0
 refs=[];sheet=Image.new('RGB',(768,220),'#303030');draw=ImageDraw.Draw(sheet)
 for i,(label,x,y)in enumerate([('sunlit-grass',1020,940),('fence-shadow-floor',1050,900),('soil-path',1020,780),('nearby-grass',1000,870)]):
  assert known[y:y+24,x:x+24].all();q=d/(label+'.png');im=Image.fromarray(source[y:y+24,x:x+24]);im.save(q);sheet.paste(im.resize((192,192),Image.Resampling.NEAREST),(192*i,28));draw.text((192*i+4,7),label,fill='white');refs.append(dict(path=str(q.resolve()),sha256=sha(q),source_bbox=[x,y,24,24],all_previously_known=True))
 sheet.save(d/'references.png')
 prompt='Fill only the indicated unseen ground beneath the initial wattle fence. Continue nearby grass, soil and existing diffuse fence shadow using the four exact same-map ground references. Do not paint any fence, post, woven wood, tree, foliage object or new prop onto the floor. Preserve the exact1792x1152 dimensions. The ordinary region guide identifies intended floor; a local authoritative mask will enforce every protected pixel. This is initial-state ground, not terminal cleared-fence artwork.'
 (d/'prompt.txt').write_text(prompt+'\n')
 (d/'request.json').write_text(json.dumps(dict(status='Proposed only; API off pending scoped geometry/domain/content review',dimensions=[1792,1152],editable_pixels=int(editable.sum()),transport='OpenRouter no-mask; ordinary region guide plus four references; local mask enforced',region_guide_sha256=sha(d/'region-guide.png'),mask_sha256=sha(d/'mask.png'),input_sha256=sha(d/'input.png'),references=refs,proposal_sha256=sha(D/'proposal.json'),initial_only_underlay=True,applied_terminal_overlay_preserved=True),indent=2)+'\n')
if __name__=='__main__':
 main();prepare_inputs()
