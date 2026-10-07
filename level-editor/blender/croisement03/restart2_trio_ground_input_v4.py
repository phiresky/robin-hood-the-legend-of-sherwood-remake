"""Prepare a narrow unapproved terrain input; never generate or change live images."""
import hashlib,io,json,struct
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];B=ROOT/'level-editor/work/croisement03-refinement/restart2';CASE=B/'trio-tree-integration-v1';OUT=CASE/'terrain-input-proposal-v4'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 OUT.mkdir(exist_ok=False);model=ROOT/'level-editor/library/3d-assets/croisement03/croisement03-terrain/model.glb';data=model.read_bytes();n=struct.unpack_from('<I',data,12)[0];g=json.loads(data[20:20+n]);v=g['bufferViews'][g['images'][0]['bufferView']];im=Image.open(io.BytesIO(data[28+n+v.get('byteOffset',0):28+n+v.get('byteOffset',0)+v['byteLength']])).convert('RGB');original=np.array(im);candidate=np.zeros(original.shape[:2],bool);guards={str(model):sha(model)}
 for tree in (12,13,14):
  p=CASE/f'ownership-resolution-v1/tree{tree}-candidate-domain.png';candidate|=np.array(Image.open(p))>0;guards[str(p)]=sha(p)
 for tree in (12,14):
  p=CASE/f'static-leaf-source-proposal-v2/tree{tree}-candidate-static-foliage.png';candidate|=np.array(Image.open(p))>0;guards[str(p)]=sha(p)
 # All temporal source positions and owned trunks belong to the receiver cleanup,
 # while their exact RGB remains protected on physical trees.
 additions=[]
 for tree in (12,14):
  case=B/f'tree{tree}-canopy-fragment-source-v1';box=json.loads((case/'scope.json').read_text())['absolute_interval'];union=np.zeros((box[3]-box[1],box[2]-box[0]),bool)
  for frame in sorted(case.glob('???.png')):
   union|=np.array(Image.open(frame).convert('RGBA'))[:,:,3]>0;guards[str(frame)]=sha(frame)
  domain=np.zeros(candidate.shape,bool);domain[box[1]:box[3],box[0]:box[2]]=union;additions.append(dict(tree=tree,role='All14 temporal positions; ground material still requires classification',new_pixels=int((domain&~candidate).sum())));candidate|=domain
 for tree in (12,13,14):
  p=B/f'tree{tree}-bark-proposal-v1/proposed-bark.png';domain=np.array(Image.open(p))>0;guards[str(p)]=sha(p);additions.append(dict(tree=tree,role='Accepted own trunk pixels on ground receiver; tree RGBA remains exact',new_pixels=int((domain&~candidate).sum())));candidate|=domain
 protected=np.zeros(candidate.shape,bool);reasons=[]
 for tree in (10,11):
  p=B/f'tree{tree}-bark-proposal-v{2 if tree==11 else 1}/proposed-bark.png'
  if p.exists():
   d=np.array(Image.open(p))>0;protected|=d;guards[str(p)]=sha(p);reasons.append(dict(kind='Known/proposed stem RGB protected',tree=tree,pixels=int((candidate&d).sum())))
 level=json.loads((B.parent/'baseline/Croisement03.rhp.json').read_text())
 for mask in (35,76,107):
  m=level['masks'][mask];a=np.array(Image.open(B.parent/f'baseline/masks/{mask:06}.png'))>0;x,y=m['box_top_left'];d=np.zeros(candidate.shape,bool);d[y:y+a.shape[0],x:x+a.shape[1]]=a;protected|=d;reasons.append(dict(kind='Reserved fern or ambiguous root/rock context',mask=mask,pixels=int((candidate&d).sum())))
 selected=candidate&~protected;Image.fromarray(selected.astype('uint8')*255).save(OUT/'candidate-unknown-domain.png');im.save(OUT/'decoded-ground-original.png');box=(936,0,1190,160);rgb=original.copy();rgb[selected]=[255,0,180];rgb[candidate&protected]=[0,220,255];left=im.crop(box).resize((762,480),Image.Resampling.NEAREST);right=Image.fromarray(rgb).crop(box).resize(left.size,Image.Resampling.NEAREST);sheet=Image.new('RGB',(1524,510),'#333333');sheet.paste(left,(0,30));sheet.paste(right,(762,30));ImageDraw.Draw(sheet).text((8,8),'Current terrain | Magenta: candidate foliage and OWN trunk receiver pixels; cyan: protected source/context',fill='white');sheet.save(OUT/'input-review.png')
 seed=np.load(CASE/'seed-ground-ownership-v2/coarse-owners.npz')['owner'];refs=[]
 for name,crop in [('moss-and-fallen-leaf-floor',(1005,195,1035,222)),('shaded-leaf-floor',(1080,330,1110,357))]:
  assert np.all(seed[crop[1]:crop[3],crop[0]:crop[2]]<0), 'Reference includes old generated ground fill'
  path=OUT/f'reference-{name}.png';im.crop(crop).save(path);refs.append(dict(file=str(path),sha256=sha(path),source='Current same-map terrain only',crop=list(crop),role='Supplementary observed floor material, no branches, foliage crowns or trunk silhouettes',prior_coarse_fill_pixels=0))
 receipt=dict(status='HOLD diagnostic receiver coverage revision; not ready for approval',terrain_model_sha256=sha(model),decoded_rgb_sha256=hashlib.sha256(original.tobytes()).hexdigest(),size=list(im.size),candidate_union_pixels=int(candidate.sum()),protected_candidate_pixels=int((candidate&protected).sum()),proposed_fill_pixels=int(selected.sum()),outside_mask_pixels_immutable=True,protect=reasons,receiver_additions=additions,source_guards=guards,supplementary_references=refs,limits=['Per-pixel accepted/provisional crown domains are a maximum candidate, not blanket rectangle permission. Material classification must confirm that selected ground pixels depict foliage rather than known floor or a neighboring prop.','All14 Arbre06 union plus traced static foliage and own accepted bark. Remaining dark/green/trunk margins remain explicit unresolved domains; no automatic dilation.','Tree13 static75 and Tree12/14 shared animated fragments retain distinct ownership limitations.','Ground model geometry, gameplay, UVs and all tree bytes must remain unchanged. Generated output would require exact local source protection and separate appearance review.','Native/opposite physical contact proof pending; publication remains held.'])
 (OUT/'scope.json').write_text(json.dumps(receipt,indent=2)+'\n');print(receipt['candidate_union_pixels'],receipt['protected_candidate_pixels'],receipt['proposed_fill_pixels'])
if __name__=='__main__':main()
