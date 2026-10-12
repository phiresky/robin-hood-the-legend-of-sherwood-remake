"""Pin potential native mask overlaps without claiming physical foreground authority."""
from pathlib import Path
import json,hashlib
import numpy as np
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];W=ROOT/'level-editor/work/croisement03-refinement/mission-logtrap-oct12-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 out=W/'foreground-qualification-v1';out.mkdir(exist_ok=False);manifest=ROOT/'datadirs/fullgame_gog_hackable/Data/Levels/Croisement03.rhp.d/masks/manifest.json';j=json.loads(manifest.read_text());base=Image.open(W/'source-map-context.png').convert('RGB').resize((300,240),Image.Resampling.NEAREST);source=np.array(Image.open(W/'terminal-aligned.png'));known=source[:,:,3]>=128;tiles=[];rows=[];union=np.zeros((240,300),bool)
 for row in j['masks']:
  x,y=row['box_top_left'];w,h=row['box_size'];l,t,r,b=max(x,420),max(y,380),min(x+w,720),min(y+h,620)
  if r<=l or b<=t:continue
  file=manifest.parent/row['png'];a=np.array(Image.open(file).convert('L'));crop=np.zeros((240,300),bool);crop[t-380:b-380,l-420:r-420]=a[t-y:b-y,l-x:r-x]>0
  if not crop.any():continue
  im=np.array(base).copy();im[crop]=np.rint(im[crop]*.4+np.array([40,140,255])*.6).astype('uint8');tile=Image.fromarray(im);draw=ImageDraw.Draw(tile);draw.text((4,4),f"global{row['index']} layer{row['layer']} local{row['layer_index']}",fill='yellow');tiles.append(tile);Image.fromarray(crop.astype('uint8')*255).save(out/f"mask-{row['index']:03}-crop.png")
  rows.append({'global_index':row['index'],'layer':row['layer'],'local_index':row['layer_index'],'path':str(file),'sha256':sha(file),'crop_pixels':int(crop.sum()),'terminal_visible_wood_overlap':int((known&crop).sum()),'character_polyline':row['character_polyline'],'own_trap_removed_mask':row['index']==125})
  if row['layer']==0 and row['index']!=125:union|=crop
 sheet=Image.new('RGB',(1200,240*((len(tiles)+3)//4)),'#222')
 for i,im in enumerate(tiles):sheet.paste(im,(i%4*300,i//4*240))
 sheet.save(out/'all-overlapping-masks.png');Image.fromarray(union.astype('uint8')*255).save(out/'potential-foreign-mask-union.png')
 report={'status':'POTENTIAL_OCCLUSION_ONLY','manifest_sha256':sha(manifest),'terminal_sha256':sha(W/'terminal-aligned.png'),'rows':rows,'limitations':['Character masks are not pixel-ownership or geometry authority.','Global125 is the removable initial trap mask and is excluded from foreign potential union.','Actual plant geometry and native source visibility must qualify inferred log continuations.','Source keyholes are not evidence of fractures or isolated chips.']};(out/'manifest.json').write_text(json.dumps(report,indent=2)+'\n');print([(r['global_index'],r['terminal_visible_wood_overlap']) for r in rows])
if __name__=='__main__':main()
