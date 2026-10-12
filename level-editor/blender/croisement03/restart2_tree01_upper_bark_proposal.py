"""Trace visible upper bark samples without promoting foliage or shadow pixels to wood."""
import json,hashlib
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/active/croisement03/restart2';O=B/'tree01-upper-bark-proposal-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 assert not O.exists();O.mkdir();path=B.parent/'baseline/covered.png';src=Image.open(path).convert('RGBA');rgb=np.array(src);native=np.zeros((960,1408),bool)
 for i in range(14):
  im=np.array(Image.open(B/'tree01-native-fragment-v1'/f'{i:03}.png').convert('RGBA'));native[:175,:175]|=im[:,:,3]>0
 static=np.array(Image.open(B/'tree01-static-leaf-proposal-v1/proposed-source-rgba.png').convert('RGBA'));native[:175,:175]|=static[:,:,3]>0
 rows=[{'role':'left connected stem','proposed_mesh_owner':3,'source_context':1003,'traces':[[(45,95),(47,106)],[(50,120),(52,127)],[(55,140),(57,148)]]},{'role':'central left upper stem','proposed_mesh_owner':3,'source_context':3,'traces':[[(82,64),(81,75)],[(81,82),(80,94)],[(79,106),(79,117)],[(78,126),(78,138)]]},{'role':'central right upper stem','proposed_mesh_owner':4,'source_context':4,'traces':[[(97,82),(99,93)],[(101,102),(103,111)],[(105,119),(107,130)]]},{'role':'right upper stem','proposed_mesh_owner':4,'source_context':5,'traces':[[(154,92),(154,102)],[(153,111),(152,122)]]}];union=np.zeros(native.shape,bool);marked=np.array(src.convert('RGB'));colors=[(255,70,210),(70,230,255),(255,190,40),(230,230,255)]
 for row,color in zip(rows,colors):
  mask=Image.new('L',src.size);draw=ImageDraw.Draw(mask)
  for line in row['traces']:draw.line(line,fill=255,width=1)
  proposal=np.array(mask)>0;filtered=proposal&~native
  # Visually traced spines remain proposals; excluding observed foliage is necessary
  # but does not by itself establish bark ownership.
  row['raw_pixels']=int(proposal.sum());row['observed_foliage_excluded']=int((proposal&native).sum());row['unassigned_proposed_pixels']=int(filtered.sum());row['candidate_pixels']=[[int(x),int(y)]for y,x in zip(*np.nonzero(filtered))];union|=filtered;marked[filtered]=color;Image.fromarray(filtered.astype('uint8')*255).save(O/(row['role'].replace(' ','-')+'.png'))
 Image.fromarray(union.astype('uint8')*255).save(O/'proposed-upper-bark.png');box=(35,50,165,160);a=src.crop(box).convert('RGB');b=Image.fromarray(marked).crop(box);sheet=Image.new('RGB',(260,110));sheet.paste(a);sheet.paste(b,(130,0));sheet.resize((1040,440),Image.Resampling.NEAREST).save(O/'source-comparison.png');(O/'proposal.json').write_text(json.dumps({'status':'UNASSIGNED_UPPER_BARK_SOURCE_PROPOSAL','source_sha256':sha(path),'total':int(union.sum()),'rows':rows,'limits':['Proposed one-pixel spines require direct source review and saved-model first hits before assignment.','All14 observed native alpha and proposed static leaf pixels excluded.','Remaining dark branches and bark widths remain unresolved; no geometry or source texture changed.']},indent=2)+'\n');print({'unassigned_upper_samples':int(union.sum()),'per_stem':[r['unassigned_proposed_pixels']for r in rows]})
if __name__=='__main__':main()
