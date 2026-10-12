"""Conservative visible bark spines; occlusion masks never assign material ownership."""
import hashlib,json
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/active/croisement03';O=B/'restart2/tree01-bark-seed-proposal-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 assert not O.exists();O.mkdir();source=B/'baseline/covered.png';src=Image.open(source).convert('RGBA')
 # Narrow disconnected samples deliberately stop before shrubs and leaf crossings.
 traces={2:[[(78,222),(81,232),(83,243),(84,250)]],3:[[(79,151),(78,162),(76,173)],[(75,188),(76,198)]],4:[[(107,140),(108,151),(110,162),(112,170)],[(115,180),(118,190),(121,202),(124,213),(128,225)]],5:[[(150,130),(149,141),(148,151)],[(147,160),(146,170),(144,181),(144,191)],[(143,201),(141,210),(140,217)]]}
 union=np.zeros((960,1408),bool);rows=[];mark=src.copy();draw=ImageDraw.Draw(mark);colors={2:'#00ffff',3:'#ff55ff',4:'#44aaff',5:'#ffffff'}
 conflict=json.loads((B/'restart2/bank-whole-prototype-v3/native-contact-v2/tree01-coarse-guard.json').read_text());conflicts=np.zeros_like(union)
 for x,y,*_ in conflict['changes']:conflicts[y,x]=True
 for node,lines in traces.items():
  mask=Image.new('L',src.size);d=ImageDraw.Draw(mask)
  for line in lines:d.line(line,fill=255,width=1);draw.line(line,fill=colors[node],width=1)
  arr=np.array(mask)>0;assert not (arr&union).any();union|=arr;mask.save(O/f'building-{node:03}-proposed-bark.png');rows.append({'source_obstacle':node,'traces':lines,'pixels':int(arr.sum()),'bank_coarse_conflict_overlap':int((arr&conflicts).sum())})
 box=(45,100,170,270);sheet=Image.new('RGB',(1000,680),'#333333')
 for i,im in enumerate((src,mark)):sheet.paste(im.crop(box).convert('RGB').resize((500,680),Image.Resampling.NEAREST),(i*500,0))
 sheet.save(O/'source-and-proposed-spines.png');Image.fromarray(union.astype('uint8')*255).save(O/'proposed-visible-bark.png')
 out={'status':'HOLD_SOURCE_PROPOSAL_NOT_OWNERSHIP','source_sha256':sha(source),'rows':rows,'total_pixels':int(union.sum()),'limits':['These narrow visible samples are a source-tracing starting point, not a complete bark domain or approval.','Four obstacle IDs remain separate. No inferred merging of stems, canopy membership or root elevation.','Stop before foliage crossings; broader silhouette and occluded continuation require independent evidence.','Mask1 includes foreground stone and is not a bark material mask.'],'next':'Use these visible segments for source-facing geometry constraints, trace silhouette widths and canopy contacts, review supplementary bark fill context after geometry approval.'}
 (O/'proposal.json').write_text(json.dumps(out,indent=2)+'\n');print(json.dumps(out))
if __name__=='__main__':main()
