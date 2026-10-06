"""Inspect existing floor synthesis only inside the residual inferred-domain proposal."""
from pathlib import Path
import json,hashlib
import numpy as np
from PIL import Image,ImageDraw
from scipy import ndimage
ROOT=Path(__file__).resolve().parents[3];OUT=ROOT/'level-editor/work/croisement02-refinement';D=OUT/'restart3-remaining-floor-reuse-review-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 D.mkdir(exist_ok=False);audit=OUT/'restart3-remaining-floor-audit-v1';report=json.loads((audit/'completion-proposal.json').read_text());domain=np.array(Image.open(audit/'proposed-inferred-floor-union.png').convert('L'))>0;assert domain.sum()==14876
 basepath=OUT/'restart3-ground-reuse-combined-v1/composite.png';rawpath=OUT/'restart2-ground-completion/approved-fill-retry-v2/generation-short-no-mask-with-lighting-openrouter-with-auxiliary/generated-raw.png';base=np.array(Image.open(basepath).convert('RGBA'));raw=np.array(Image.open(rawpath).convert('RGBA'));preview=base.copy();preview[domain,:3]=raw[domain,:3];assert np.array_equal(preview[~domain],base[~domain]);Image.fromarray(preview).save(D/'proposed-appearance.png')
 source=np.array(Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA'));allgray=np.all(np.array(Image.open(OUT/'restart2-state/underlay-input-review-v1/proposed-appearance.png').convert('RGBA'))[:,:,:3]==127,axis=2)
 for p in ['known.png','separate_relief.png']:allgray&=~(np.array(Image.open(OUT/'restart2-ground-completion/preparation-v1'/p).convert('L'))>0)
 for p in [OUT/'restart3-initial-fence/floor-proposal-v2/inferred-hidden-floor.png',OUT/'restart3-initial-fence/tree45-floor-proposal-v1/inferred-floor-domain.png',OUT/'restart2-state/underlay-aggregate-audit-v3/combined-candidate-domain.png']:allgray&=~(np.array(Image.open(p).convert('L'))>0)
 labels,n=ndimage.label(allgray,np.ones((3,3)));records=[]
 for group in report['groups']:
  if group['dominant_source_context']=='unassigned-fringe':continue
  mask=np.isin(labels,group['components'])&domain;yy,xx=np.nonzero(mask);box=(max(0,int(xx.min())-12),max(0,int(yy.min())-12),min(1792,int(xx.max())+13),min(1152,int(yy.max())+13));w=box[2]-box[0];h=box[3]-box[1];scale=min(4,720//max(w,h));scale=max(1,scale)
  over=source.copy();over[mask,:3]=(over[mask,:3]*.3+np.array([235,140,35])*.7).astype('uint8');sheet=Image.new('RGB',(w*scale*3,h*scale+32),'#303030');draw=ImageDraw.Draw(sheet)
  for i,(title,a)in enumerate([('Source / orange inferred floor',over),('Current saved868 floor',base),('Exact existing raw reuse preview',preview)]):sheet.paste(Image.fromarray(a).convert('RGB').crop(box).resize((w*scale,h*scale),Image.Resampling.NEAREST),(i*w*scale,32));draw.text((i*w*scale+4,8),title,fill='white')
  f=D/f"source-{group['dominant_source_context']}-reuse.png";sheet.save(f);records.append(dict(source_context=group['dominant_source_context'],pixels=int(mask.sum()),box=box,image=str(f),image_sha256=sha(f)))
 (D/'manifest.json').write_text(json.dumps(dict(status='Private raw reuse suitability review; no model or API change',base_model_sha256='868cf916e4b8d7e808262974396f825080e053aac9173923fa513a1c12012929',base_atlas_sha256=sha(basepath),base_appearance_pending=True,raw_response=str(rawpath),raw_sha256=sha(rawpath),domain_sha256=sha(audit/'proposed-inferred-floor-union.png'),editable_pixels=14876,all_outside_rgba_exact=True,alpha_exact=True,held49_unedited=True,preview_sha256=sha(D/'proposed-appearance.png'),records=records),indent=2)+'\n');print(D)
if __name__=='__main__':main()
