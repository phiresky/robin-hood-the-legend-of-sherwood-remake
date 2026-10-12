"""Report candidate stone-sample first hits without assigning source ownership."""
import json,hashlib,math
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
from restart2_bank_boundary_ray import boundary_hits
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/croisement03-refinement/restart2';O=B/'bank-face-source-proposal-v1';S=math.sin(math.radians(35));C=math.cos(math.radians(35));RAY=np.array([0.,-C,S])
def main():
 output=O/'cpu-firsthits-v1.json';assert not output.exists();paths=[B/'bank-boulder-fronts-cpu-v8/geometry.json',B/'bank-boulder-east-cpu-v8/geometry.json',B/'bank-continuous-strata-plan-v1/geometry.json'];data,east,west=[json.loads(p.read_text())for p in paths];mask=np.array(Image.open(O/'proposed-native-stone.png'))>0;owner=Image.new('L',(mask.shape[1],mask.shape[0]));draw=ImageDraw.Draw(owner)
 for row in json.loads((B/'bank-visible-face-traces-v1/faces.json').read_text())['faces']:draw.polygon([tuple(p)for p in row['native_outline']],fill=row['owner'])
 expected=np.array(owner);pair=lambda m:(m['vertices'],m['faces']);counts={};bad=[]
 for y,x in np.argwhere(mask):
  meshes={'53':pair(west),'54':pair(data['54']),'52-east':pair(east)}
  if x+.5<=east['x_cut']:meshes['52-main']=pair(data['52'])
  hits=boundary_hits(np.array([x+.5,-(y+.5)/S,0.])+RAY*5000,-RAY,meshes);actual=int(hits[0]['owner'].split('-')[0])if hits else None;counts[str(actual)]=counts.get(str(actual),0)+1
  if actual!=int(expected[y,x]):bad.append({'source':[int(x),int(y)],'proposed':int(expected[y,x]),'firsthit':actual})
 report={'status':'CPU_CANDIDATE_SOURCE_HITS_REVIEW_ONLY','inputs':{str(p):hashlib.sha256(p.read_bytes()).hexdigest()for p in paths},'proposal_sha256':hashlib.sha256((O/'proposal.json').read_bytes()).hexdigest(),'samples':int(mask.sum()),'firsthit_owners':counts,'different_proposed_owner':bad,'limits':['Geometric first-hit consistency is not visual source ownership proof.','Saved Boolean, neighboring foreground layers and protected source checks remain mandatory before assignment.','All candidate samples remain unassigned.']};output.write_text(json.dumps(report,indent=2)+'\n');print({'samples':report['samples'],'owners':counts,'mismatches':len(bad)})
if __name__=='__main__':main()
