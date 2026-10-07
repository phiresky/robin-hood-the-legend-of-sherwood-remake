"""Restore a finite rectangular timber section independently of pixel-fit score."""
import ast,copy,json,math
from pathlib import Path
import numpy as np
from shapely.geometry import Polygon
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement/restart2';OUT=WORK/'timber-sawn-end-plan-v1'
if OUT.exists():raise FileExistsError(OUT)
code=Path(__file__).with_name('restart22_timber_corrected_plan.py').read_text();code=code.replace("vertices=np.array([[x,y,z]for z in(p['bottom_z'],p['top_z'])for x,y in p['footprint_world']])","vertices=np.array([[x,y,p['bottom_z']]for x,y in p.get('bottom_footprint_world',p['footprint_world'])]+[[x,y,p['top_z']]for x,y in p['footprint_world']])")
exec(compile(ast.Module(body=[n for n in ast.parse(code).body if isinstance(n,ast.FunctionDef)],type_ignores=[]),'<sawn-end ray audit>','exec'))
s=math.sin(math.radians(35));c=math.cos(math.radians(35));back=np.array([0,-c,s]);faces=[(0,3,2,1),(4,5,6,7),(0,1,5,4),(1,2,6,5),(2,3,7,6),(3,0,4,7)]
base=json.loads((WORK/'timber-full-fit-v1/plan.json').read_text());parts=copy.deepcopy(base['pieces']);long=next(p for p in parts if p['id']=='long-crossing');observed=long['top_source_polygon'];cx=sum(x for x,y in observed)/4;cy=sum(y for x,y in observed)/4;sw=[-1,1,1,-1];sl=[-1,-1,1,1];u=[sum(q[k]*v for q,v in zip(observed,sw))/4 for k in(0,1)];v=[sum(q[k]*z for q,z in zip(observed,sl))/4 for k in(0,1)];best=None
for i in range(-899,900):
 t=math.radians(i/10);a=[math.cos(t),-s*math.sin(t)];b=[-math.sin(t),-s*math.cos(t)];width=sum(x*y for x,y in zip(u,a))/sum(x*x for x in a);length=sum(x*y for x,y in zip(v,b))/sum(x*x for x in b)
 points=[[cx+x*width*a[0]+y*length*b[0],cy+x*width*a[1]+y*length*b[1]]for x,y in zip(sw,sl)];errors=[math.dist(x,y)for x,y in zip(observed,points)]
 if best is None or max(errors)<best['maximum_pixel_error']:best={'maximum_pixel_error':max(errors),'errors':errors,'pixels':points,'width_world':abs(width*2),'length_world':abs(length*2),'angle_degrees':i/10}
assert best['maximum_pixel_error']<2
long['top_source_polygon']=best['pixels'];long['footprint_world']=[[x,-(y+c*long['top_z'])/s]for x,y in best['pixels']]
for p in parts:p['bottom_footprint_world']=copy.deepcopy(p['footprint_world'])
a=np.array(long['footprint_world']);width=a[1]-a[0];length=a[3]-a[0];assert abs(float(width@length))<1e-7
source=json.loads((WORK/'timber-observed-domains-v1/report.json').read_text());pixels=[r['pixel']for r in source['pixels']];expected=[tuple(r['owner'])for r in source['pixels']];actual=ray_owners(parts,pixels);missing=[{'pixel':xy,'expected':e,'actual':a}for xy,e,a in zip(pixels,expected,actual)if a!=e]
contacts=[]
for a in parts:
 for b in parts:
  if a['id']==b['id']or abs(a['top_z']-b['bottom_z'])>1e-6:continue
  overlap=Polygon(a['footprint_world']).intersection(Polygon(b['bottom_footprint_world']))
  if overlap.area>1e-8:contacts.append({'lower':a['id'],'upper':b['id'],'area':overlap.area})
report={'status':'PRIVATE_FINITE_SAWN_END_GEOMETRY','ground_plane_z':base['ground_plane_z'],'pieces':parts,'long_section':{'width':best['width_world'],'height':long['top_z']-long['bottom_z'],'end_face_area':best['width_world']*(long['top_z']-long['bottom_z']),'perpendicular_end_cut':True,'bevel':0,'taper':0},'source_corner_fit':best,'independent_visible_pixels':len(pixels),'matched_cpu':len(pixels)-len(missing),'missing':missing,'bearing_contacts':contacts,'limitations':['The four old source corners were approximate top/end observations, not authority for an acute timber cut.','All973 visible wood targets retained, even where rectangular geometry does not fit them.','Saved-model topology, source and oblique review still required; no approval inferred.']};OUT.mkdir();(OUT/'plan.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({'fit':best,'section':report['long_section'],'matched':report['matched_cpu'],'contacts':len(contacts)}))
