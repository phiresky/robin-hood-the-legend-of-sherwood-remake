"""Freeze narrow hand-traced visible bark proposals independently of leaf masks."""
import json,hashlib
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/croisement03-refinement';OUT=BASE/'restart2/tree13-bark-proposal-v1'
POLYGONS={48:[(1045,59),(1048,60),(1048,69),(1046,72),(1045,84),(1043,85),(1043,75),(1044,68)],31:[(1069,46),(1071,46),(1072,58),(1070,62),(1068,59),(1068,51)],32:[(1089,58),(1091,59),(1090,67),(1088,75),(1086,78),(1086,72),(1088,65)]}
def main():
 OUT.mkdir(exist_ok=False);source=Image.open(BASE/'baseline/covered.png').convert('RGB');rgb=np.array(source);level=json.loads((BASE/'baseline/Croisement03.rhp.json').read_text());domain=Image.new('L',source.size);domain.paste(Image.open(BASE/'baseline/masks/000013.png'),tuple(level['masks'][13]['box_top_left']));native=np.array(domain)>0;union=np.zeros(native.shape,bool);rows=[]
 for node,polygon in POLYGONS.items():
  region=Image.new('L',source.size);ImageDraw.Draw(region).polygon(polygon,fill=255);selected=(np.array(region)>0)&native
  # Hand-traced cores remain proposals. Exclude bright yellow/green leaf flecks
  # inside these tiny local regions rather than classifying the entire tree.
  local_bark=(rgb[:,:,0].astype(int)>=rgb[:,:,1].astype(int))&(rgb[:,:,1].astype(int)-rgb[:,:,2].astype(int)<55)&(rgb.max(2)<200)
  selected&=local_bark
  if node==48:selected|=np.array(Image.open(BASE/'restart2/fern-wood-proposal-v1/35-proposed-wood.png'))>0
  union|=selected;file=OUT/f'node-{node:03}-proposed-bark.png';Image.fromarray(selected.astype('uint8')*255).save(file);rows.append({'node':node,'trace_polygon':polygon,'proposed_pixels':int(selected.sum()),'mask_sha256':hashlib.sha256(file.read_bytes()).hexdigest()})
 Image.fromarray(union.astype('uint8')*255).save(OUT/'proposed-bark.png');marked=rgb.copy();marked[union]=[255,0,255];box=(1030,25,1110,110);a=source.crop(box).resize((640,680),Image.Resampling.NEAREST);b=Image.fromarray(marked).crop(box).resize(a.size,Image.Resampling.NEAREST);sheet=Image.new('RGB',(1280,680));sheet.paste(a,(0,0));sheet.paste(b,(640,0));sheet.save(OUT/'proposal-comparison.png');(OUT/'proposal.json').write_text(json.dumps({'status':'PRIVATE PROPOSAL; not accepted source ownership or texture fill','source_sha256':hashlib.sha256((BASE/'baseline/covered.png').read_bytes()).hexdigest(),'items':rows,'total_proposed_pixels':int(union.sum()),'limits':['Hand-traced narrow bark cores with local leaf-fleck exclusion only.','No model or texture altered; source ownership requires visual and ray review.','Other native mask13 pixels remain unresolved mixed domains, including static/animated foliage.']},indent=2)+'\n');print(rows)
if __name__=='__main__':main()
