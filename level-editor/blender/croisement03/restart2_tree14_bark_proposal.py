"""Trace conservative native bark cores while preserving approved fern receiver pixels."""
import json,hashlib
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];B=ROOT/'level-editor/work/croisement03-refinement';OUT=B/'restart2/tree14-bark-proposal-v1'
def main():
 OUT.mkdir(exist_ok=False);src=Image.open(B/'baseline/covered.png').convert('RGB');d=json.loads((B/'baseline/Croisement03.rhp.json').read_text());mask=Image.new('L',src.size);mask.paste(Image.open(B/'baseline/masks/000014.png'),tuple(d['masks'][14]['box_top_left']));polys=[[(1126,31),(1128,31),(1126,46),(1124,57),(1122,69),(1120,80),(1119,91),(1117,103),(1115,109),(1115,98),(1117,86),(1118,73),(1120,62),(1122,51)],[(1135,34),(1137,34),(1135,47),(1134,57),(1133,67),(1131,78),(1131,86),(1129,85),(1130,72),(1131,59),(1132,46)]];candidate=Image.new('L',src.size);draw=ImageDraw.Draw(candidate)
 for poly in polys:draw.polygon(poly,fill=255)
 a=np.array(candidate)>0;rgb=np.array(src).astype(int);a&=np.array(mask)>0;a&=(rgb[:,:,0]>=rgb[:,:,1])&((rgb[:,:,1]-rgb[:,:,2])<45)&(rgb.max(axis=2)<210)
 k=np.zeros(a.shape,bool);a|=k;Image.fromarray(np.where(a,255,0).astype('uint8')).save(OUT/'proposed-bark.png');box=(1110,15,1160,145);base=src.crop(box);overlay=np.array(src);overlay[a]=[255,40,170];sheet=Image.new('RGB',(600,625));sheet.paste(base.resize((300,625),Image.Resampling.NEAREST),(0,0));sheet.paste(Image.fromarray(overlay).crop(box).resize((300,625),Image.Resampling.NEAREST),(300,0));sheet.save(OUT/'proposal-comparison.png');(OUT/'proposal.json').write_text(json.dumps({'status':'Private positive bark proposal; root visual review required','accepted_prior_pixels':0,'proposed_total':int(a.sum()),'new_pixels':int((a&~k).sum()),'polygons':polys,'limits':['Two visible bark cores; green ivy and bottom boulder excluded.','Two coarse native nodes do not prove whether the stems join; keep source-supported curved forms and disclose unknown upper continuation.','All unselected foliage, ivy, dark background and ground remain unassigned.']},indent=2)+'\n');print(a.sum())
if __name__=='__main__':main()
