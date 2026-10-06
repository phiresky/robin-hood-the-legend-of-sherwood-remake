"""Trace conservative native bark cores while preserving approved fern receiver pixels."""
import json,hashlib
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];B=ROOT/'level-editor/work/croisement03-refinement';OUT=B/'restart2/tree12-bark-proposal-v1'
def main():
 OUT.mkdir(exist_ok=False);src=Image.open(B/'baseline/covered.png').convert('RGB');d=json.loads((B/'baseline/Croisement03.rhp.json').read_text());mask=Image.new('L',src.size);mask.paste(Image.open(B/'baseline/masks/000012.png'),tuple(d['masks'][12]['box_top_left']));polys=[[(981,91),(984,91),(984,102),(985,110),(986,126),(987,139),(984,143),(982,128),(981,111)],[(1000,94),(1002,94),(1001,105),(1000,115),(998,128),(998,136),(996,141),(996,128),(998,115),(999,103)],[(1009,95),(1011,95),(1010,108),(1008,115),(1007,127),(1006,139),(1005,148),(1003,148),(1004,131),(1005,119),(1008,106)]];candidate=Image.new('L',src.size);draw=ImageDraw.Draw(candidate)
 for poly in polys:draw.polygon(poly,fill=255)
 a=np.array(candidate)>0;rgb=np.array(src).astype(int);a&=np.array(mask)>0;a&=(rgb[:,:,0]>=rgb[:,:,1])&((rgb[:,:,1]-rgb[:,:,2])<45)&(rgb.max(axis=2)<210)
 known=Image.new('L',src.size);draw=ImageDraw.Draw(known);rows=json.loads((B/'restart2/fern-receiver-audit-v1/audit.json').read_text())['items'];prior=next(r for r in rows if r['fern_mask']==76)['pixels']
 for p in prior:draw.point((p['x'],p['y']),fill=255)
 k=np.array(known)>0;assert k.sum()==64;a|=k;Image.fromarray(np.where(a,255,0).astype('uint8')).save(OUT/'proposed-bark.png');box=(975,70,1035,195);base=src.crop(box);overlay=np.array(src);overlay[a]=[255,40,170];sheet=Image.new('RGB',(600,625));sheet.paste(base.resize((300,625),Image.Resampling.NEAREST),(0,0));sheet.paste(Image.fromarray(overlay).crop(box).resize((300,625),Image.Resampling.NEAREST),(300,0));sheet.save(OUT/'proposal-comparison.png');(OUT/'proposal.json').write_text(json.dumps({'status':'Private positive bark proposal; root visual review required','accepted_prior_pixels':64,'proposed_total':int(a.sum()),'new_pixels':int((a&~k).sum()),'polygons':polys,'limits':['Three visible narrow bark tracks; joining topology is unresolved under foreground fern.','Prior coarse node024 attribution is not proof of final branch ownership; preserve exact64 source pixels in complete tree12 group.','All unselected foliage, ivy, dark background and ground remain unassigned.']},indent=2)+'\n');print(a.sum())
if __name__=='__main__':main()
