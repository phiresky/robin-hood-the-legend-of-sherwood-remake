"""Prepare inspectable native stone samples within traced faces; do not assign materials."""
import json,hashlib
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/active/croisement03/restart2';O=B/'bank-face-source-proposal-v1'
def main():
 assert not O.exists();O.mkdir();path=B.parent/'baseline/covered.png';src=Image.open(path).convert('RGB');rgb=np.array(src).astype(int);rows=json.loads((B/'bank-visible-face-traces-v1/faces.json').read_text())['faces'];old=np.array(Image.open(B/'bank-exposed-rock-proposal-v1/candidate-rock-seeds.png'))>0;r,g,b=rgb[:,:,0],rgb[:,:,1],rgb[:,:,2];neutral=(b>=40)&(r>=70)&(g>=60)&(abs(r-g)<=60)&(g-b<=55)&(r>=g*.9);union=np.zeros(old.shape,bool);counts=[]
 for row in rows:
  mask=Image.new('L',src.size);ImageDraw.Draw(mask).polygon([tuple(p)for p in row['native_outline']],fill=255);candidate=(np.array(mask)>0)&neutral&~old;union|=candidate;counts.append({'face':row['id'],'proposed_owner':row['owner'],'samples':int(candidate.sum()),'source_role_status':'Visual stone-interior proposal, unassigned; any remaining foreground twig/leaf ambiguity must be rejected.'})
 Image.fromarray(union.astype('uint8')*255).save(O/'proposed-native-stone.png')
 for name,box in [('east',(548,183,636,356)),('south',(335,333,426,448))]:
  marked=np.array(src);marked[union]=[0,255,255];raw=src.crop(box);overlay=Image.fromarray(marked).crop(box);sheet=Image.new('RGB',(raw.width*2,raw.height));sheet.paste(raw);sheet.paste(overlay,(raw.width,0));sheet.resize((sheet.width*5,sheet.height*5),Image.Resampling.NEAREST).save(O/(name+'-proposal.png'))
 report={'status':'UNASSIGNED_SOURCE_PROPOSAL_REQUIRES_DIRECT_REVIEW','source_sha256':hashlib.sha256(path.read_bytes()).hexdigest(),'face_proposal_sha256':hashlib.sha256((B/'bank-visible-face-traces-v1/faces.json').read_bytes()).hexdigest(),'existing3446samples_sha256':hashlib.sha256((B/'bank-exposed-rock-proposal-v1/candidate-rock-seeds.png').read_bytes()).hexdigest(),'new_samples':int(union.sum()),'overlap_existing':int((union&old).sum()),'faces':counts,'rule':'Bright neutral/warm source samples within independently traced visible boulder face regions; saturated foliage excluded as a proposal only.','limits':['No source ownership or material changed.','Color filtering is not ownership proof; compare every region against original art and source layers before assigning.','Gray surface remainder stays unknown; no inferred RGB or API generation.']};(O/'proposal.json').write_text(json.dumps(report,indent=2)+'\n');print(report['new_samples'])
if __name__=='__main__':main()
