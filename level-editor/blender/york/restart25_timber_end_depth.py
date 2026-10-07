"""CPU test of a planar tapered long timber end with the traced top fixed."""
import ast,copy,json,math
from pathlib import Path
import numpy as np
from shapely.geometry import Polygon
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement/restart2';OUT=WORK/'timber-end-depth-v1'
if OUT.exists():raise FileExistsError(OUT)
code=Path(__file__).with_name('restart22_timber_corrected_plan.py').read_text();code=code.replace("vertices=np.array([[x,y,z]for z in(p['bottom_z'],p['top_z'])for x,y in p['footprint_world']])","vertices=np.array([[x,y,p['bottom_z']]for x,y in p.get('bottom_footprint_world',p['footprint_world'])]+[[x,y,z]for(x,y),z in zip(p['footprint_world'],p.get('top_vertex_z',[p['top_z']]*4))])")
exec(compile(ast.Module(body=[n for n in ast.parse(code).body if isinstance(n,ast.FunctionDef)],type_ignores=[]),'<tapered top ray audit>','exec'))
s=math.sin(math.radians(35));c=math.cos(math.radians(35));back=np.array([0,-c,s]);faces=[(0,3,2,1),(4,5,6,7),(0,1,5,4),(1,2,6,5),(2,3,7,6),(3,0,4,7)]
base=json.loads((WORK/'timber-edge-fit-v2/plan.json').read_text());src=json.loads((WORK/'timber-observed-domains-v1/report.json').read_text());pixels=[r['pixel']for r in src['pixels']];expected=[tuple(r['owner'])for r in src['pixels']];trials=[];best=None
for growth in[0,.5,1,1.5,2,2.5,3]:
 for inset in[.25,.5,.75,1]:
  parts=copy.deepcopy(base['pieces']);p=next(p for p in parts if p['id']=='long-crossing');start=(p['top_source_polygon'][0][0]+p['top_source_polygon'][1][0])/2;end=(p['top_source_polygon'][2][0]+p['top_source_polygon'][3][0])/2
  p['top_vertex_z']=[p['top_z']+growth*(x-start)/(end-start)for x,y in p['top_source_polygon']];p['footprint_world']=[[x,-(y+c*z)/s]for(x,y),z in zip(p['top_source_polygon'],p['top_vertex_z'])];p['bottom_footprint_world']=copy.deepcopy(p['footprint_world'])
  for i in(0,3):p['bottom_footprint_world'][i][1]+=inset/s
  for i in(2,3):p['bottom_footprint_world'][i][1]-=.5/s
  owners=ray_owners(parts,pixels);matched=sum(a==e for a,e in zip(owners,expected));trial={'long_end_growth_world':growth,'near_bottom_inset_px':inset,'matched':matched};trials.append(trial)
  if best is None or matched>best['matched']:best={**trial,'pieces':parts,'owners':owners}
contacts=[]
for a in best['pieces']:
 for b in best['pieces']:
  if a['id']==b['id']or abs(a['top_z']-b['bottom_z'])>1e-6:continue
  overlap=Polygon(a['footprint_world']).intersection(Polygon(b.get('bottom_footprint_world',b['footprint_world'])))
  if overlap.area>1e-8:contacts.append({'lower':a['id'],'upper':b['id'],'area':overlap.area})
missing=[{'pixel':xy,'expected':e,'actual':a}for xy,e,a in zip(pixels,expected,best.pop('owners'))if e!=a];report={'status':'PRIVATE_PLANAR_END_DEPTH_PROPOSAL','fit':{k:v for k,v in best.items()if k!='pieces'},'pieces':best['pieces'],'ground_plane_z':base['ground_plane_z'],'trials':trials,'missing':missing,'bearing_contacts':contacts,'limits':['Varying long-timber depth is an inferred hidden shape, not a measured source dimension.','Top plane varies linearly with sourceX, retaining every traced source top corner.','No saved-model or oblique validation yet; do not trade construction quality for pixel-fit score.']};OUT.mkdir();(OUT/'plan.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({'fit':report['fit'],'contacts':len(contacts),'missing':len(missing)}))
