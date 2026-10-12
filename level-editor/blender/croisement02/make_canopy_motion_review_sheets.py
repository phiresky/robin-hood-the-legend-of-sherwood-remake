"""Prepare labeled review sheets from existing saved renders and pinned native frames."""
from pathlib import Path
from PIL import Image,ImageDraw
import json,argparse,hashlib
p=argparse.ArgumentParser();p.add_argument('--group',type=int,required=True);p.add_argument('--packet',type=Path,required=True);a=p.parse_args();B=Path('level-editor/work/croisement02-refinement');O=B/f'canopy{a.group:02}-independent-motion-review-oct12-v1';O.mkdir(exist_ok=False);r=json.loads((a.packet/'request.json').read_text());row=r['rows'][0];frames=row.get('native_source_frames',r.get('native_source_frames'));captures=a.packet/'captures-v1';prefix=f'canopy-group-{a.group:02}';calibrated=(captures/f'{prefix}-native-bbox-tick-0.png').exists();pins={}
def load(path):
 pins[str(path)]=hashlib.sha256(path.read_bytes()).hexdigest();im=Image.open(path).convert('RGBA');bg=Image.new('RGBA',im.size,(37,37,37,255));bg.alpha_composite(im);return bg.convert('RGB')
def tile(out,path,x,y,w,h,label):
 im=load(path);im.thumbnail((w,h-24));out.paste(im,(x+(w-im.width)//2,y+24));ImageDraw.Draw(out).text((x+4,y+4),label,fill='white')
for page in range(3):
 out=Image.new('RGB',(800,5*385),(45,45,45))
 for j,phase in enumerate(range(page*5,min(15,page*5+5))):
  tile(out,Path(frames[phase%14]['path']),0,j*385,400,385,f'Native phase{phase%14}')
  f=f'{prefix}-native-bbox-tick-{phase*4}.png'if calibrated else f'{prefix}-tick-{phase*4}-view-0.png'
  tile(out,captures/f,400,j*385,400,385,f'Actual tick{phase*4}; '+('nativebbox'if calibrated else 'fullbounds original direction'))
 out.save(O/f'phase-pairs-{page}.jpg')
out=Image.new('RGB',(4*384,2*410),(45,45,45))
for i in range(8):
 f=f'{prefix}-native-bbox-tick-28.png'if calibrated and i==0 else f'{prefix}-tick-28-view-{i}.png';tile(out,captures/f,(i%4)*384,(i//4)*410,384,410,'Original art direction'if i==0 else f'Direction{i}')
out.save(O/'eight-views.jpg')
for tree in row['tree_ids']:
 names=[]
 if calibrated:
  for tick,mode in [(0,'actual'),(28,'actual'),(0,'transparent'),(28,'transparent'),(28,'cutaway')]:
   for direction in ['original','reverse']:names.append((captures/f'{prefix}-tree-{tree}-{mode}-{direction}-tick-{tick}.png',f'{tree} {mode} {direction} tick{tick}'))
 else:
  for tick in [0,28]:
   for direction in ['original','reverse']:names.append((captures/f'{prefix}-tree-{tree}-contact-{direction}-tick-{tick}.png',f'{tree} actual {direction} tick{tick}'))
 out=Image.new('RGB',(768,410*((len(names)+1)//2)),(45,45,45))
 for i,(path,label)in enumerate(names):tile(out,path,(i%2)*384,(i//2)*410,384,410,label)
 out.save(O/f'tree-{tree}-junctions.jpg')
(O/'input-pins.json').write_text(json.dumps(pins,indent=2)+'\n');print(O)
