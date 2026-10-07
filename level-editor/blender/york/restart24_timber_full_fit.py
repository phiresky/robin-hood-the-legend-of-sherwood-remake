"""Fit bounded timber side depths against the full visible-face trace."""
import ast,copy,hashlib,json,math
from pathlib import Path
import numpy as np
from shapely.geometry import Polygon
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement/restart2';OUT=WORK/'timber-full-fit-v1'
if OUT.exists():raise FileExistsError(OUT)
recipe=Path(__file__).with_name('restart22_timber_corrected_plan.py');exec(compile(ast.Module(body=[n for n in ast.parse(recipe.read_text()).body if isinstance(n,ast.FunctionDef)],type_ignores=[]),str(recipe),'exec'))
s=math.sin(math.radians(35));c=math.cos(math.radians(35));back=np.array([0,-c,s]);faces=[(0,3,2,1),(4,5,6,7),(0,1,5,4),(1,2,6,5),(2,3,7,6),(3,0,4,7)]
source=json.loads((WORK/'timber-observed-domains-v1/report.json').read_text());base=json.loads((WORK/'timber-corrected-plan-v1/plan.json').read_text());pixels=[r['pixel']for r in source['pixels']];expected=[tuple(r['owner'])for r in source['pixels']];ground=base['ground_plane_z'];best=None;trials=[]
for pale in[3,4,5,6]:
 for long in[2,3,4,5]:
  for short in[3,4,5,6]:
   pieces=copy.deepcopy(base['pieces'])
   for p in pieces:
    if p['id'].startswith('pale'):p['bottom_z']=ground;p['top_z']=ground+pale
    else:p['bottom_z']=ground+pale;p['top_z']=ground+pale+(long if p['id']=='long-crossing'else short)
    p['footprint_world']=[[x,-(y+c*p['top_z'])/s]for x,y in p['top_source_polygon']]
   owners=ray_owners(pieces,pixels);matched=sum(a==e for a,e in zip(owners,expected));trial={'pale':pale,'long':long,'short':short,'matched':matched};trials.append(trial)
   if best is None or matched>best['matched']:best={**trial,'pieces':pieces,'owners':owners}
contacts=[];overlaps=[]
for i,a in enumerate(best['pieces']):
 for b in best['pieces'][i+1:]:
  intersection=Polygon(a['footprint_world']).intersection(Polygon(b['footprint_world']));depth=min(a['top_z'],b['top_z'])-max(a['bottom_z'],b['bottom_z'])
  if intersection.area<1e-8:continue
  if depth>1e-6:overlaps.append({'a':a['id'],'b':b['id'],'area':intersection.area,'depth':depth})
  elif abs(depth)<1e-6:contacts.append({'a':a['id'],'b':b['id'],'area':intersection.area})
missing=[{'pixel':xy,'expected':e,'actual':a}for xy,e,a in zip(pixels,expected,best.pop('owners'))if e!=a]
report={'status':'PRIVATE_FULL_SOURCE_FIT_PROPOSAL','pieces':best.pop('pieces'),'ground_plane_z':ground,'parameters':best,'source_domains_sha256':hashlib.sha256((WORK/'timber-observed-domains-v1/report.json').read_bytes()).hexdigest(),'trials':trials,'independent_visible_pixels':len(pixels),'missing':missing,'bearing_contacts':contacts,'interpenetrations':overlaps,'limits':['Chosen thicknesses are fitted hypotheses; actual saved contact queries and all views required.','Full source trace is independent of accepted texture pixels; missing wood remains a target.','23 overlapping domain pixels remain excluded pending boundary review.']}
OUT.mkdir();(OUT/'plan.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({'parameters':best,'pixels':len(pixels),'missing':len(missing),'contacts':len(contacts),'overlaps':overlaps}))
