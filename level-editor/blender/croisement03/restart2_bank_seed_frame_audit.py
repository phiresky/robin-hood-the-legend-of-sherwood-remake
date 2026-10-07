"""Measure temporal alpha overlap of proposed bank source seeds without assigning it."""
import hashlib,json
from pathlib import Path
import numpy as np
from PIL import Image
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/croisement03-refinement';O=B/'restart2/bank-exposed-rock-proposal-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 mp=B/'animation-references/manifest.json';sp=O/'candidate-rock-seeds.png';domain=np.array(Image.open(sp))>0;ys,xs=np.nonzero(domain);box=[int(xs.min()),int(ys.min()),int(xs.max()+1),int(ys.max()+1)];records=[];union=np.zeros(domain.shape,bool)
 for animation in json.loads(mp.read_text())['animations']:
  frames=[];seen=np.zeros(domain.shape,bool)
  for i,f in enumerate(animation['frames']):
   x,y,w,h=f['bbox']
   if x>=box[2] or y>=box[3] or x+w<=box[0] or y+h<=box[1]:continue
   path=Path(f['image']);raw=Path(f['source']);assert sha(raw)==f['sha256'];rgba=np.array(Image.open(path).convert('RGBA'));assert rgba.shape[:2]==(h,w)
   left=max(0,x);top=max(0,y);right=min(domain.shape[1],x+w);bottom=min(domain.shape[0],y+h)
   hit=np.zeros(domain.shape,bool);hit[top:bottom,left:right]=(rgba[top-y:bottom-y,left-x:right-x,3]>0)&domain[top:bottom,left:right];seen|=hit
   frames.append(dict(index=i,saved_sha256=sha(path),source_sha256=sha(raw),overlapping_seed_pixels=int(hit.sum())))
  if frames:records.append(dict(profile=animation['profile'],frames=frames,ever_overlapping_seed_pixels=int(seen.sum())));union|=seen
 assert not (O/'saved-frame-overlap.json').exists()
 Image.fromarray(union.astype('uint8')*255).save(O/'seed-temporal-overlap.png')
 report=dict(status='Saved alpha placement diagnostic; no runtime ordering or source ownership approval',manifest_sha256=sha(mp),seed_sha256=sha(sp),candidate_pixels=int(domain.sum()),ever_covered_pixels=int(union.sum()),never_covered_pixels=int((domain&~union).sum()),animations=records,limits=['Raw frame alpha retains its actual checkerboard sampling. No occupancy hull replaces appearance alpha.','Overlap does not authorize deleting static bank surfaces or known source RGB.','The geometry candidate must retain bank source and animated foreground as separate owners; physical/runtime layering remains pending.'])
 (O/'saved-frame-overlap.json').write_text(json.dumps(report,indent=2)+'\n');print({k:v for k,v in report.items() if k not in ('animations','limits')})
if __name__=='__main__':main()
