"""Fit small bottom-edge bevels while retaining the observed top contours."""
import ast,copy,json,math
from pathlib import Path
import numpy as np
from shapely.geometry import Polygon
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement/restart2';OUT=WORK/'timber-edge-fit-v2'
if OUT.exists():raise FileExistsError(OUT)
code=Path(__file__).with_name('restart22_timber_corrected_plan.py').read_text();code=code.replace("vertices=np.array([[x,y,z]for z in(p['bottom_z'],p['top_z'])for x,y in p['footprint_world']])","vertices=np.array([[x,y,p['bottom_z']]for x,y in p.get('bottom_footprint_world',p['footprint_world'])]+[[x,y,p['top_z']]for x,y in p['footprint_world']])")
exec(compile(ast.Module(body=[n for n in ast.parse(code).body if isinstance(n,ast.FunctionDef)],type_ignores=[]),'<beveled ray audit>','exec'))
s=math.sin(math.radians(35));c=math.cos(math.radians(35));back=np.array([0,-c,s]);faces=[(0,3,2,1),(4,5,6,7),(0,1,5,4),(1,2,6,5),(2,3,7,6),(3,0,4,7)]
base=json.loads((WORK/'timber-full-fit-v1/plan.json').read_text());source=json.loads((WORK/'timber-observed-domains-v1/report.json').read_text());pixels=[r['pixel']for r in source['pixels']];expected=[tuple(r['owner'])for r in source['pixels']];best=None;trials=[]
for inset in[0,.5]:
 for extend in[0,.5,1]:
  for short in[0,.5]:
   parts=copy.deepcopy(base['pieces']);valid=True
   for p in parts:
    bottom=copy.deepcopy(p['footprint_world'])
    if p['id']=='long-crossing':
     for i in(0,3):bottom[i][1]+=inset/s
     for i in(2,3):bottom[i][1]-=extend/s
    if p['id']=='short-crossing':
     for i in(2,3):bottom[i][1]+=short/s
    poly=Polygon(bottom)
    if not poly.is_valid or poly.area<5:valid=False;break
    p['bottom_footprint_world']=bottom
   if not valid:continue
   owners=ray_owners(parts,pixels);matches=sum(a==b for a,b in zip(owners,expected));trial={'long_near_bottom_inset_px':inset,'long_end_bottom_extension_px':extend,'short_near_bottom_inset_px':short,'matched':matches};trials.append(trial)
   if best is None or matches>best['matched']:best={**trial,'pieces':parts,'owners':owners}
contacts=[]
for a in best['pieces']:
 for b in best['pieces']:
  if a['id']==b['id']or abs(a['top_z']-b['bottom_z'])>1e-7:continue
  region=Polygon(a['footprint_world']).intersection(Polygon(b['bottom_footprint_world']))
  if region.area>1e-8:contacts.append({'lower':a['id'],'upper':b['id'],'area':region.area})
missing=[{'pixel':xy,'expected':e,'actual':a}for xy,e,a in zip(pixels,expected,best.pop('owners'))if e!=a]
report={'status':'PRIVATE_BEVELED_EDGE_PROPOSAL','ground_plane_z':base['ground_plane_z'],'pieces':best.pop('pieces'),'fit':best,'trials':trials,'independent_visible_pixels':len(pixels),'missing':missing,'bearing_contacts':contacts,'limits':['Bottom corner shifts are shape hypotheses constrained by visible edge pixels, not measured hidden cuts.','Actual saved triangulation, source re-projection, ground queries and multi-view shape review remain required.','Contact areas do not certify continuous noninterpenetration or mechanical stability.']};OUT.mkdir();(OUT/'plan.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({'fit':best,'contacts':len(contacts),'missing':missing}))
