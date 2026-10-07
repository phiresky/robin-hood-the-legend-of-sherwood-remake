"""CPU source traces and bounded shelf-ring proposal; no mesh or ownership mutation."""
import json,hashlib,math
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/croisement03-refinement';O=B/'restart2/bank-morphology-trace-v1'
S=math.sin(math.radians(35));C=math.cos(math.radians(35))

# These are visible source crease samples, not traced vegetation silhouettes.
# Paired front/bottom points retain x so a short rock face can be reconstructed
# without assigning its source footprint to a full-height vertical column.
SHELVES=[
 dict(id='west-upper-cap',color='#ffbb66',native_anchor=[179,285],
      rim=[[157,278],[165,283],[178,289],[188,286],[198,281]],
      lower=[[157,284],[165,290],[178,298],[188,296],[198,290]],
      undulation=[-1,0,1.5,.5,-1],confidence='Visible cap outline; its left shoulder meets unclassified shrub/rock pixels.'),
 dict(id='west-middle-slab',color='#66eeff',native_anchor=[175,320],
      rim=[[128,318],[142,323],[155,322],[170,318],[184,316],[197,312]],
      lower=[[128,324],[142,333],[155,333],[170,333],[184,327],[197,322]],
      undulation=[-1,0,1.5,2,1,-1],confidence='Bright continuous shelf crease and darker front band; hanging ivy below remains unclassified.'),
 dict(id='west-lower-slab',color='#ff77cc',native_anchor=[163,371],
      rim=[[128,376],[140,378],[151,374],[164,369],[178,365],[193,359]],
      lower=[[128,382],[140,389],[151,386],[164,383],[178,379],[193,370]],
      undulation=[-1,0,1.5,2,1,-1],confidence='Visible lower ledge; endpoint continuation behind ivy is inferred and excluded from traced ownership.'),
]
MAIN_EDGES=[
 dict(id='bank-left-upper-break',points=[[150,222],[160,231],[176,243],[192,248],[206,244],[221,237]],confidence='Observed exposed upper-left shoulder; stop before gold foliage.'),
 dict(id='bank-left-lower-break',points=[[159,245],[171,255],[184,263],[198,258],[215,250]],confidence='Observed short ledge beneath shoulder; right continuation obscured.'),
 dict(id='bank-right-exposed-break',points=[[605,211],[613,212],[610,230],[605,244],[608,256]],confidence='Exposed narrow right stone boundary; do not extend through adjacent dark gaps or ivy.'),
]

def main():
 O.mkdir(exist_ok=True);assert not (O/'recipe.json').exists();source=B/'baseline/covered.png';level=B/'baseline/Croisement03.rhp.json';lp=json.loads(level.read_text());points=lp['sight_obstacles'][53]['points'];a=np.array([[p['x'],p['y']-p['z_top'],1] for p in points]);heights=np.array([p['z_top'] for p in points]);plane=np.linalg.lstsq(a,heights,rcond=None)[0]
 records=[]
 for shelf in SHELVES:
  z=float(np.dot(plane,[*shelf['native_anchor'],1]));rings=[]
  for upper,lower,bump in zip(shelf['rim'],shelf['lower'],shelf['undulation']):
   x,y=upper;bottom_y=lower[1];top_z=z+bump;bottom_z=top_z-(bottom_y-y)
   top=[x,-(y+top_z)/S,top_z/C];bottom=[x,-(bottom_y+bottom_z)/S,bottom_z/C]
   assert abs(top[1]-bottom[1])<1e-6 and top_z>bottom_z>0
   rings.append(dict(source_rim=upper,source_lower=lower,upper_world=top,lower_world=bottom,inferred_height_units=[top_z,bottom_z]))
  records.append({**shelf,'inferred_anchor_height':z,'proposed_short_face_rings':rings})
 src=Image.open(source).convert('RGB');marked=src.copy();d=ImageDraw.Draw(marked)
 for shelf in SHELVES:
  for key in ['rim','lower']:d.line([tuple(p) for p in shelf[key]],fill=shelf['color'],width=1)
 for edge in MAIN_EDGES:d.line([tuple(p) for p in edge['points']],fill='white',width=1)
 box=(120,165,625,415);sheet=Image.new('RGB',(1010,1040),'#222222');draw=ImageDraw.Draw(sheet)
 for i,(label,im) in enumerate([('Native source; hidden shelf continuation is not traced',src),('Paired short shelf bands: gold/cyan/pink; exposed main-bank breaks: white',marked)]):
  sheet.paste(im.crop(box).resize((1010,500),Image.Resampling.NEAREST),(0,520*i+20));draw.text((4,520*i+3),label,fill='white')
 sheet.save(O/'shelf-source-traces.png')
 pins=[source,level,B/'restart2/geometry-round15-tree02-shared-ridge-v1/freeze.json',B/'restart2/bank-full-prototype-v2/worker.blend']
 recipe=dict(status='CPU morphology proposal; independent trace review and mesh construction pending',input_sha256={str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in pins},shelves=records,main_bank_observed_breaks=MAIN_EDGES,
  distinction={'observed':'Only visible 2D crease/edge samples listed above; no new pixel ownership is approved.','inferred':'Shelf heights anchored to authored ramp53 plane, shallow undulation, short face depth, bevels and occluded side/back closure. The source camera does not measure these heights uniquely.'},
  construction_order=['Keep the frozen shared crestx175–340 untouched and preserve native gameplay metadata verbatim.',
   'Replace only the coarse western visual ramp skin with three joined shallow shelf bands; keep its support volume private until overlap/closure checks pass.',
   'Use paired upper/lower rings for short front rock faces, with small bevel transition rings; avoid extruding each screen column down to ground.',
   'Join successive shelf bands with irregular sloped rear/top patches. Preserve source ray coordinates at the traced creases; infer hidden lateral closures without cutting at mask boundaries.',
   'On bank52, split the exposed left shoulder and narrow right edge at the traced breaks. Stop modeled source claims where gold canopy or ivy hides the continuation.',
   'Treat broad bank top relief as inference constrained by the fixed crest and first-hit guards, not an RGB displacement map.',
   'Reassign proposed rock seeds through saved-model first hits after geometry changes; all other surfaces stay neutral until material classification.'],
  mandatory_guards=['Tree02/03 accepted source rays and fixed shared crest','Tree04–07 accepted source and canopy domains','Native3446 rock seed projection','Tree01 visible bark once classified; the575 foreground-rock coarse rays are not bark authority','West path94–97 geometry/UV and gameplay metadata unchanged','Closed topology, no internal overlap curtains, native plus8oblique and west-contact saved-material review'],
  limits=['No render/model lane used. No mesh, native RGBA, approval card, gallery or canonical asset changed.','Crease coordinates have approximately1–3source-pixel uncertainty; inspect the next saved model before readiness.','Tree01 bark/crown classification and full front ivy/stone ownership remain unfinished.'])
 (O/'recipe.json').write_text(json.dumps(recipe,indent=2)+'\n');assert sum(p.stat().st_size for p in O.iterdir())<3*1024**2;print(O/'recipe.json')
if __name__=='__main__':main()
