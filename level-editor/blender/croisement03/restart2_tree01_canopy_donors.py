"""Prepare own-native golden leaf donors without assigning shared animation pixels."""
import json,hashlib
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/active/croisement03';O=B/'restart2/tree01-canopy-donors-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 inv=B/'restart2/tree01-canopy-source-v1/canopy-inputs.json';packet=json.loads(inv.read_text());frame=Path(packet['frames'][0]['path']);assert sha(frame)==packet['frames'][0]['sha256'];im=Image.open(frame).convert('RGBA');rgba=np.array(im);candidates=[]
 # These hand-scoped regions lie over the golden crown above the four woody
 # tracks. The neighboring red crown and lower green shrub are excluded.
 regions=[(20,8,76,96),(76,0,140,66),(110,60,180,126)]
 for x0,y0,x1,y1 in regions:
  for y in range(y0,y1-15,8):
   for x in range(x0,x1-15,8):
    tile=rgba[y:y+16,x:x+16];alpha=tile[:,:,3]>0;rgb=tile[:,:,:3].astype(float);gold=alpha&(rgb[:,:,0]>rgb[:,:,2]*1.4)&(rgb[:,:,1]>rgb[:,:,2]*1.25)
    if alpha.sum()>=48 and gold.sum()/alpha.sum()>=.95:candidates.append((int(alpha.sum()),x,y))
 selected=[]
 for count,x,y in sorted(candidates,reverse=True):
  if all(abs(x-r[1])>=16 or abs(y-r[2])>=16 for r in selected):selected.append((count,x,y))
  if len(selected)==12:break
 assert len(selected)>=6,'Insufficient own-native donor evidence';O.mkdir(exist_ok=False);sheet=Image.new('RGBA',(512,((len(selected)+3)//4)*148),'#333333');draw=ImageDraw.Draw(sheet);rows=[]
 for i,(count,x,y)in enumerate(selected):
  tile=im.crop((x,y,x+16,y+16));path=O/f'leaf-{i:02}.png';tile.save(path);assert np.array_equal(np.array(Image.open(path)),rgba[y:y+16,x:x+16]);ox=i%4*128;oy=i//4*148;sheet.alpha_composite(tile.resize((128,128),Image.Resampling.NEAREST),(ox,oy));draw.text((ox+2,oy+130),f'{i:02} x{x} y{y}',fill='white');rows.append({'path':str(path),'sha256':sha(path),'native_box':[x,y,x+16,y+16],'alpha_pixels':count,'rgba_exact_crop':True})
 sheet.convert('RGB').save(O/'donor-contact-sheet.png');report={'status':'PRIVATE_NATIVE_DONORS_ONLY','source_frame_sha256':sha(frame),'source_inventory_sha256':sha(inv),'regions':regions,'donors':rows,'observed_native_owner':'Shared Arbre08; these crops authorize no duplication of observed animation roles.','use':'Only inferred hidden leaf volume or supplementary fill context after applicable geometry/input review.','limits':['Checkerboard native alpha remains unchanged; occupancy masks are never texture RGBA.','No full crown geometry or source ownership approval.','External tree references remain limited to the two named Leicester trees.']};(O/'packet.json').write_text(json.dumps(report,indent=2)+'\n');print({'donors':len(rows),'status':report['status']})
if __name__=='__main__':main()
