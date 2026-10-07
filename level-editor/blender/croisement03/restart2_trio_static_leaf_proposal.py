"""Unapproved static leaf candidates adjacent to observed animated leaf samples."""
from pathlib import Path
import hashlib,json
import numpy as np
from PIL import Image,ImageFilter
ROOT=Path(__file__).resolve().parents[3];B=ROOT/'level-editor/work/croisement03-refinement/restart2';OUT=B/'trio-tree-integration-v1/static-leaf-source-proposal-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 OUT.mkdir(exist_ok=False);source=B.parent/'baseline/covered.png';native=Image.open(source).convert('RGB');rgb=np.array(native).astype(int);reserved=np.zeros(rgb.shape[:2],bool);level=json.loads((B.parent/'baseline/Croisement03.rhp.json').read_text())
 for tree in (10,11,12,13,14):
  p=B/f'tree{tree}-bark-proposal-v{2 if tree==11 else 1}/proposed-bark.png'
  if p.exists():reserved|=np.array(Image.open(p))>0
 for i in (35,76,107):
  m=level['masks'][i];x,y=m['box_top_left'];a=np.array(Image.open(B.parent/f'baseline/masks/{i:06}.png'))>0;reserved[y:y+a.shape[0],x:x+a.shape[1]]|=a
 rows=[]
 for tree in (12,14):
  case=B/f'tree{tree}-canopy-fragment-source-v1';scope=json.loads((case/'scope.json').read_text());x,y,u,v=scope['absolute_interval'];frames=[np.array(Image.open(p))[:,:,3]>0 for p in sorted(case.glob('???.png'))];union=np.any(frames,axis=0);near=np.array(Image.fromarray(union.astype('uint8')*255).filter(ImageFilter.MaxFilter(3)))>0;c=rgb[y:v,x:u];gold=(c[:,:,0]-c[:,:,2]>=25)&(c[:,:,1]-c[:,:,2]>=15)&(c[:,:,0]>=c[:,:,1]*.8);candidate=near&~union&gold&~reserved[y:v,x:u];domain=np.zeros(reserved.shape,bool);domain[y:v,x:u]=candidate;Image.fromarray(domain.astype('uint8')*255).save(OUT/f'tree{tree}-candidate-static-leaf.png');a=c.astype('uint8');marked=a.copy();marked[union]=[255,0,180];marked[candidate]=[0,220,255];sheet=Image.new('RGB',((u-x)*2,v-y));sheet.paste(Image.fromarray(a),(0,0));sheet.paste(Image.fromarray(marked),(u-x,0));sheet.resize((sheet.width*4,sheet.height*4),Image.Resampling.NEAREST).save(OUT/f'tree{tree}-proposal-comparison.png');rows.append(dict(tree=tree,box=[x,y,u,v],known_14frame_union_pixels=int(union.sum()),candidate_additional_static_pixels=int(candidate.sum()),method='Native gold/brown high-chroma pixels at most one source pixel from observed14frame leaf support, excluding all accepted/proposed bark and reserved fern/root context',status='PROPOSAL requires direct material/source review; not accepted ownership'))
 (OUT/'scope.json').write_text(json.dumps(dict(status='PRIVATE SOURCE PROPOSAL, not input-ready',source_sha256=sha(source),rows=rows,limits=['Candidate classifier can include tiny brown twigs or distant foliage; no automatic acceptance from color or alpha adjacency.','Does not establish exhaustive crown silhouette; unselected background remains unresolved.','No source pixels transferred, no geometry/material changed, no API.','If accepted, added physical static leaf source samples require fresh geometry review before terrain fill.']),indent=2)+'\n');print(rows)
if __name__=='__main__':main()
