"""Pin exact native mask evidence for disputed private bank receiver samples."""
import argparse,hashlib,json
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/croisement03-refinement';W=B/'restart2'
def main():
 p=argparse.ArgumentParser();p.add_argument('--output',type=Path,required=True);args=p.parse_args();assert not args.output.exists();pins={}
 def read(path):pins[str(path)]=hashlib.sha256(path.read_bytes()).hexdigest();return json.loads(path.read_text())
 def image(path):pins[str(path)]=hashlib.sha256(path.read_bytes()).hexdigest();return Image.open(path)
 prior=read(W/'bank53-source-retention-v1/receipt.json');rows=prior['failures'];assert len(rows)==102;q=np.array([r['source_xy']for r in rows]);manifest=read(B/'baseline/masks/manifest.json');level=read(B/'baseline/Croisement03.rhp.json');hits=[]
 for m in manifest['masks']:
  a=np.array(image(B/'baseline/masks'/m['png']));xy=q-m['box_top_left'];inside=(xy[:,0]>=0)&(xy[:,1]>=0)&(xy[:,0]<a.shape[1])&(xy[:,1]<a.shape[0]);hit=np.zeros(len(q),bool);hit[inside]=a[xy[inside,1],xy[inside,0]]>0
  if hit.any():hits.append({'metadata':m,'disputed_coverage_count':int(hit.sum()),'source_xy':q[hit].tolist()})
 assert [(h['metadata']['index'],h['disputed_coverage_count'])for h in hits]==[(96,102)]
 covered=image(B/'baseline/covered.png').convert('RGB');revealed=image(B/'baseline/revealed.png').convert('RGB');ca,ra=np.array(covered),np.array(revealed);same=np.all(ca[q[:,1],q[:,0]]==ra[q[:,1],q[:,0]],axis=1)
 patch_links=[]
 for i,patch in enumerate(level['patches']):
  for field in ('old_masks','new_masks'):
   for ref in patch[field]:
    if any(ref['layer']==h['metadata']['layer']and ref['index']==h['metadata']['layer_index']for h in hits):patch_links.append({'patch':i,'field':field,'reference':ref})
 crop=(132,255,174,295);scale=12;original=covered.crop(crop).resize((504,480),Image.Resampling.NEAREST);annotated=original.copy();draw=ImageDraw.Draw(annotated)
 # Manual visual review guides only: these are deliberately not assignment masks.
 guides=[{'label':'Right A stone cheek (observed)','points':[[156,264],[162,260],[168,263],[165,276],[158,283],[154,278]],'color':'#00ffff'}, {'label':'Dark seam / ivy (uncertain)','points':[[149,260],[155,260],[154,272],[150,283],[146,284],[147,273]],'color':'#ffcc00'}]
 for guide in guides:
  ps=[((x-crop[0])*scale,(y-crop[1])*scale)for x,y in guide['points']];draw.line(ps+[ps[0]],fill=guide['color'],width=2)
 sheet=Image.new('RGB',(1008,535),'#242424');sheet.paste(original,(0,30));sheet.paste(annotated,(504,30));d=ImageDraw.Draw(sheet);d.text((5,8),'Unmarked covered source',fill='white');d.text((510,8),'Visual review guides; NOT pixel ownership',fill='white');d.text((5,515),'Cyan: continuous stone cheek. Yellow: dark cleft / ivy, ambiguous. Guides approximate.',fill='white')
 args.output.mkdir();original.save(args.output/'original-12x.png');sheet.save(args.output/'source-continuity-12x.png')
 receipt={'status':'HOLD_102_PRIVATE_OWNER_DISAGREEMENTS_NO_NATIVE_COMPONENT_BOUNDARY','pins':pins,'disputed_count':102,'native_mask_hits':hits,'covered_revealed_same_rgb_samples':int(same.sum()),'patch_links_to_covering_masks':patch_links,'visual_guides':guides,'observations':['All102 samples are covered only by global96, layer0, masktype7. Its obstacle_indices is empty. Native mask coverage establishes shared bank occlusion, not a52/53 subdivision.','The right-hand rounded stone cheek has continuous highlighted texture and a curved shoulder into A; some disputed samples lie on this visible stone interior.','A narrow dark cleft separates that cheek from a smaller left stone face. Other disputed samples lie in the shadow/ivy transition; source appearance alone does not determine their depth or asset owner.','The current bank-only receiver test has no gaps.52 rather than53 is a disagreement with a previous private proxy-derived hypothesis, not independent evidence that stone has disappeared.','Neither old source-band marks nor the manual A outline establishes ownership of the entire disputed patch.'],'decision':['Retain all102 samples and all prior source/protected lists unchanged pending root review.','Do not force a53 bulge to recover oldproxy labels. Compare joined saved source projection and material continuity across52/53 before any ownership or shape decision.','Keep protected3446 and1834trace checks strict. Check these additional102 samples as explicit unresolved source continuity evidence.'],'limits':['CPU bank receiver scope only; unchanged94–97 exact saved triangles unavailable.','Visual guides are rough observational annotations, not segmentation, source masks or geometry anchors.','No geometry, source images, masks, owners or canonical files changed.']}
 (args.output/'receipt.json').write_text(json.dumps(receipt,indent=2)+'\n');print(json.dumps({k:receipt[k]for k in ['status','covered_revealed_same_rgb_samples','patch_links_to_covering_masks']}))
if __name__=='__main__':main()
