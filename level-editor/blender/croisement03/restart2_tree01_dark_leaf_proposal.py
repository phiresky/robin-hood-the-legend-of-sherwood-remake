"""Inspect dark complementary leaf samples without treating all dark pixels as foliage."""
import json,hashlib
from pathlib import Path
import numpy as np
from PIL import Image
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/croisement03-refinement/restart2';O=B/'tree01-dark-static-proposal-v1'
def main():
 assert not O.exists();O.mkdir();source=B.parent/'baseline/covered.png';src=np.array(Image.open(source).convert('RGBA'))[:175,:175];fragment=B/'tree01-native-fragment-v1';frame=np.array(Image.open(fragment/'000.png').convert('RGBA'));union=np.array(Image.open(fragment/'alpha-union.png'))>0;known=np.array(Image.open(B/'tree01-static-leaf-proposal-v1/candidate.png'))>0;wood=np.array(Image.open(B/'tree01-static-leaf-proposal-v1/excluded-wood-projection.png'))>0;domain=np.array(Image.open(fragment/'spatial-proposal.png'))>0;best=np.full((175,175),999)
 for dy,dx in [(0,1),(0,-1),(1,0),(-1,0)]:
  shifted=np.roll(frame,(dy,dx),(0,1));valid=shifted[:,:,3]>0
  if dy==1:valid[0]=False
  if dy==-1:valid[-1]=False
  if dx==1:valid[:,0]=False
  if dx==-1:valid[:,-1]=False
  distance=np.max(np.abs(src[:,:,:3].astype(int)-shifted[:,:,:3].astype(int)),axis=2);best=np.minimum(best,np.where(valid,distance,999))
 candidate=(best<=8)&~union&~known&~wood&domain&(src[:,:,:3].max(2)<65);assert not(candidate&union).any()and not(candidate&wood).any();Image.fromarray(candidate.astype('uint8')*255).save(O/'candidate.png');rgba=src.copy();rgba[:,:,3]=candidate.astype('uint8')*255;Image.fromarray(rgba).save(O/'proposed-rgba.png');marked=src[:,:,:3].copy();marked[candidate]=[0,255,255];sheet=Image.new('RGB',(350,175));sheet.paste(Image.fromarray(src[:,:,:3]));sheet.paste(Image.fromarray(marked),(175,0));sheet.resize((1050,525),Image.Resampling.NEAREST).save(O/'comparison.png');report={'status':'PRIVATE_PROPOSAL_UNASSIGNED','source_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'native_frame_sha256':hashlib.sha256((fragment/'000.png').read_bytes()).hexdigest(),'candidate_pixels':int(candidate.sum()),'rule':'Exact source dark pixel, maxRGB<65, differs by at most8/channel from adjacent opaque native frame0 leaf; outside14frame alpha union, proposed staticgold and projectedwood.','limits':['Adjacency and color continuity are proposals, not proof of static ownership.','This does not classify source complement as background or authorize deleting dark foliage.','No geometry, texture, terrain receiver or canonical ownership changes.']};(O/'receipt.json').write_text(json.dumps(report,indent=2)+'\n');print(report['candidate_pixels'])
if __name__=='__main__':main()
