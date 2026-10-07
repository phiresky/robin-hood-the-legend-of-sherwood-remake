"""CPU-only native ridge section evidence; no tree or terrain model mutation."""
import json, math, hashlib
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw
R=Path(__file__).resolve().parents[3]; B=R/'level-editor/work/croisement03-refinement'; O=B/'restart2/tree02-ridge-section-proposal-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 O.mkdir(exist_ok=False); level=json.loads((B/'baseline/Croisement03.rhp.json').read_text()); source=Image.open(B/'baseline/covered.png').convert('RGB'); masks={}
 for n in (2,3,96):
  im=Image.new('L',source.size); im.paste(Image.open(B/f'baseline/masks/{n:06}.png'),tuple(level['masks'][n]['box_top_left']));masks[n]=np.array(im)>0
 pts=level['sight_obstacles'][52]['points']; a,b=pts[1:3]
 def edge(x):
  t=(x-a['x'])/(b['x']-a['x']); return (a['y']+t*(b['y']-a['y']),a['z_top']+t*(b['z_top']-a['z_top']))
 samples=[]
 for x in range(190,226):
  yy,z=edge(x+.5); inds=np.flatnonzero(masks[96][130:220,x]);samples.append(dict(x=x,map_y=yy,height=z,projected_crest_y=yy-z,first_mask96_y=int(inds[0]+130) if len(inds) else None))
 bark=np.array(Image.open(B/'restart2/tree02-bark-proposal-v1/proposed-bark.png'))>0;other=np.array(Image.open(B/'restart2/tree03-bark-proposal-v1/proposed-bark.png'))>0
 # Projected overlap is a candidate risk, not a physical first-hit result.
 risks={}
 for name,domain in [('tree02_bark',bark),('tree03_bark',other)]:
  rows=[]
  for y,x in zip(*np.nonzero(domain)):
   if a['x']<=x+.5<=b['x']:
    yy,z=edge(x+.5)
    if y+.5>=yy-z:rows.append([int(x),int(y),bool(masks[96][y,x])])
  risks[name]=rows
 box=(160,105,345,245); marked=np.array(source)
 marked[masks[96]]=(marked[masks[96]]*.5+np.array([0,220,255])*.5).astype('uint8'); marked[bark]=[255,40,170];marked[other]=[255,210,20]
 marked=Image.fromarray(marked);draw=ImageDraw.Draw(marked)
 draw.line([(x,edge(x)[0]-edge(x)[1]) for x in range(185,337)],fill=(255,255,255),width=1)
 sheet=Image.new('RGB',(1110,460),'#222222');d=ImageDraw.Draw(sheet)
 for i,im in enumerate((source,marked)):
  sheet.paste(im.crop(box).resize((555,420),Image.Resampling.NEAREST),(555*i,30))
 d.text((10,8),'Original native pixels',(255,255,255));d.text((565,8),'Cyan mask96; white authored crest; magenta Tree02; gold Tree03',(255,255,255));sheet.save(O/'native-ridge-comparison.png')
 # Section in authored mapY / vertical-height units at x211.
 yy,z=edge(211); section=Image.new('RGB',(900,520),'#222222');d=ImageDraw.Draw(section)
 def p(y,h):return (int(70+(y-210)*5),int(450-h*3))
 d.polygon([p(yy,0),p(yy,z),p(360,z),p(360,0)],fill='#685847',outline='cyan');d.line([p(238,0),p(238,137)],fill='#bc9370',width=8)
 for sy,color in [(166,'#ff66bb'),(200,'#bbbbbb'),(238,'#eeeeee')]:
  d.line([p(218,218-sy),p(345,345-sy)],fill=color,width=2);d.text(p(320,320-sy),f' source y={sy}',fill=color)
 d.text((20,15),'x=211: authored obstacle52 top85, rear edge mapY250.97; Tree02 base mapY238',(255,255,255));d.text((20,40),'Native rays satisfy screenY = mapY - height. Bank is in front of lower trunk.',(255,255,255));d.text((20,65),'Diagram is evidence proposal; raster edge and physical neighbor occlusion still need validation.',(255,255,255));section.save(O/'cross-section.png')
 pins=[B/'baseline/Croisement03.rhp.json',B/'baseline/covered.png',B/'baseline/masks/000096.png',B/'restart2/tree02-isolated-prototype-v7/worker.blend']
 result=dict(status='CPU section proposal; exact saved-model receiver guard pending',source_hashes={str(p.relative_to(R)):sha(p) for p in pins},obstacle52=level['sight_obstacles'][52],tree02_obstacle6=level['sight_obstacles'][6],samples=samples,source_foot_y=166,section_x=211,projected_crest_y=yy-z,tree02_base_map_y=238,proposed_interpretation='Tree02 remains on lower ground behind the authored raised plateau. Its lower trunk is hidden by the foreground ridge, rather than moved onto the plateau.',mask96_layer=level['masks'][96]['layer'],mask96_character_threshold=level['masks'][96]['character_polyline'],mask96_projectile_threshold=level['masks'][96]['projectile_polyline'],projected_overlap_risks=risks,limits=['Mask threshold fields select sprite occlusion and are not geometric elevations. Authored obstacle52 supplies height independently.','Source skyline is irregular; the collision edge is not automatically the exact rendered ridge edge.','Tree03 and all source first-hit rays must be checked against a compact actual obstacle52 section before geometry readiness.','No images or meshes have been modified; all appearances remain native evidence.'])
 (O/'proposal.json').write_text(json.dumps(result,indent=2)+'\n');assert sum(p.stat().st_size for p in O.iterdir())<2*1024**2
 print(json.dumps(dict(output=str(O),crest=yy-z,risks={k:len(v) for k,v in risks.items()},mask_samples=samples[18:24])))
if __name__=='__main__':main()
