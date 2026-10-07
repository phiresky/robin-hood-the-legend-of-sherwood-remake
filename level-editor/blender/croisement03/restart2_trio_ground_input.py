"""Prepare a narrow unapproved terrain input; never generate or change live images."""
import hashlib,io,json,struct
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];B=ROOT/'level-editor/work/croisement03-refinement/restart2';CASE=B/'trio-tree-integration-v1';OUT=CASE/'terrain-input-proposal-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 OUT.mkdir(exist_ok=False);model=ROOT/'level-editor/library/3d-assets/croisement03/croisement03-terrain/model.glb';data=model.read_bytes();n=struct.unpack_from('<I',data,12)[0];g=json.loads(data[20:20+n]);v=g['bufferViews'][g['images'][0]['bufferView']];im=Image.open(io.BytesIO(data[28+n+v.get('byteOffset',0):28+n+v.get('byteOffset',0)+v['byteLength']])).convert('RGB');original=np.array(im);candidate=np.zeros(original.shape[:2],bool);guards={str(model):sha(model)}
 for tree in (12,13,14):
  p=CASE/f'ownership-resolution-v1/tree{tree}-candidate-domain.png';candidate|=np.array(Image.open(p))>0;guards[str(p)]=sha(p)
 protected=np.zeros(candidate.shape,bool);reasons=[]
 for tree in (10,11,12,13,14):
  p=B/f'tree{tree}-bark-proposal-v{2 if tree==11 else 1}/proposed-bark.png'
  if p.exists():
   d=np.array(Image.open(p))>0;protected|=d;guards[str(p)]=sha(p);reasons.append(dict(kind='Known/proposed stem RGB protected',tree=tree,pixels=int((candidate&d).sum())))
 level=json.loads((B.parent/'baseline/Croisement03.rhp.json').read_text())
 for mask in (35,76,107):
  m=level['masks'][mask];a=np.array(Image.open(B.parent/f'baseline/masks/{mask:06}.png'))>0;x,y=m['box_top_left'];d=np.zeros(candidate.shape,bool);d[y:y+a.shape[0],x:x+a.shape[1]]=a;protected|=d;reasons.append(dict(kind='Reserved fern or ambiguous root/rock context',mask=mask,pixels=int((candidate&d).sum())))
 selected=candidate&~protected;Image.fromarray(selected.astype('uint8')*255).save(OUT/'candidate-unknown-domain.png');im.save(OUT/'decoded-ground-original.png');box=(936,0,1190,160);rgb=original.copy();rgb[selected]=[255,0,180];rgb[candidate&protected]=[0,220,255];left=im.crop(box).resize((762,480),Image.Resampling.NEAREST);right=Image.fromarray(rgb).crop(box).resize(left.size,Image.Resampling.NEAREST);sheet=Image.new('RGB',(1524,510),'#333333');sheet.paste(left,(0,30));sheet.paste(right,(762,30));ImageDraw.Draw(sheet).text((8,8),'Current terrain | Magenta: candidate foliage only; cyan: protected source/context',fill='white');sheet.save(OUT/'input-review.png')
 refs=[]
 for name,crop in [('moss-and-fallen-leaf-floor',(1005,195,1035,222)),('exposed-earth-and-leaf-litter',(947,282,978,308)),('shaded-leaf-floor',(1080,330,1110,357))]:
  path=OUT/f'reference-{name}.png';im.crop(crop).save(path);refs.append(dict(file=str(path),sha256=sha(path),source='Current same-map terrain only',crop=list(crop),role='Supplementary floor material, no branches, foliage crowns or trunk silhouettes'))
 receipt=dict(status='INPUT PROPOSAL ONLY; no synthesis or appearance approval',terrain_model_sha256=sha(model),decoded_rgb_sha256=hashlib.sha256(original.tobytes()).hexdigest(),size=list(im.size),candidate_union_pixels=int(candidate.sum()),protected_candidate_pixels=int((candidate&protected).sum()),proposed_fill_pixels=int(selected.sum()),outside_mask_pixels_immutable=True,protect=reasons,source_guards=guards,supplementary_references=refs,limits=['Per-pixel accepted/provisional crown domains are a maximum candidate, not blanket rectangle permission. Material classification must confirm that selected ground pixels depict foliage rather than known floor or a neighboring prop.','Arbre06 frame0 domain alone may leave other-phase painted fringe; no automatic dilation or expansion.','Tree13 static75 and Tree12/14 shared animated fragments retain distinct ownership limitations.','Ground model geometry, gameplay, UVs and all tree bytes must remain unchanged. Generated output would require exact local source protection and separate appearance review.','Native/opposite physical contact proof pending; publication remains held.'])
 (OUT/'scope.json').write_text(json.dumps(receipt,indent=2)+'\n');print(receipt['candidate_union_pixels'],receipt['protected_candidate_pixels'],receipt['proposed_fill_pixels'])
if __name__=='__main__':main()
