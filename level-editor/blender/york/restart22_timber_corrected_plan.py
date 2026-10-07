"""CPU source-ray fit for separate boards; does not modify frozen candidates."""
import copy,hashlib,json,math
from pathlib import Path
import numpy as np
from shapely.geometry import Polygon,Point
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement/restart2';OUT=WORK/'timber-corrected-plan-v2'
if OUT.exists():raise FileExistsError(OUT)
source=json.loads((WORK/'loose-planks-ownership-v3/report.json').read_text());old=json.loads((WORK/'loose-planks-candidate-v1/candidate.json').read_text());pieces=copy.deepcopy(old['pieces']);s=math.sin(math.radians(35));c=math.cos(math.radians(35));back=np.array([0,-c,s])
for p in pieces:
 domain=next(d for d in source['domains']if d['piece']==p['id']and d['role']=='top')
 p['top_source_polygon']=domain['polygon'];p['footprint_world']=[[x,-(y+c*p['top_z'])/s]for x,y in domain['polygon']]
 p['correction']='Top edges follow independently traced visible top region; bottom/end/side surfaces derive separately from prism thickness.'
long_piece=next(p for p in pieces if p['id']=='long-crossing')
for i in (2,3):long_piece['top_source_polygon'][i][1]+=2
long_piece['footprint_world']=[[x,-(y+c*long_piece['top_z'])/s]for x,y in long_piece['top_source_polygon']]
faces=[(0,3,2,1),(4,5,6,7),(0,1,5,4),(1,2,6,5),(2,3,7,6),(3,0,4,7)]
def ray_owners(parts,pixels):
 origins=np.array([[x+.5,-(y+.5)/s,0]for x,y in pixels])+back*1000;direction=-back;nearest=np.full(len(origins),np.inf);owners=[None]*len(origins)
 for p in parts:
  vertices=np.array([[x,y,z]for z in(p['bottom_z'],p['top_z'])for x,y in p['footprint_world']])
  for fi,face in enumerate(faces):
   for indices in ((face[0],face[1],face[2]),(face[0],face[2],face[3])):
    a,b,d=vertices[list(indices)];e1=b-a;e2=d-a;h=np.cross(direction,e2);det=np.dot(e1,h)
    if abs(det)<1e-9:continue
    q=origins-a;u=q@h/det;cross=np.cross(q,e1);v=cross@direction/det;t=cross@e2/det
    hit=(u>=-1e-8)&(v>=-1e-8)&(u+v<=1+1e-8)&(t>=0)&(t<nearest)
    for i in np.flatnonzero(hit):owners[i]=(p['id'],'top'if fi==1 else 'bottom'if fi==0 else 'side')
    nearest[hit]=t[hit]
 return owners
missing=source['independent_wood_not_matched'];pixels=[r['pixel']for r in missing];expected=[tuple(r['owner'])for r in missing];side_trace=Polygon([(1264,930),(1291,958),(1292,963),(1264,935)])
reassigned=[]
for i,((x,y),owner) in enumerate(zip(pixels,expected)):
 if owner[0].startswith('pale') and side_trace.covers(Point(x+.5,y+.5)):
  reassigned.append({'pixel':[x,y],'old_owner':owner,'corrected_owner':['long-crossing','side'],'reason':'Native brown side strip was omitted from top-versus-side trace.'});expected[i]=('long-crossing','side')
before=ray_owners(old['pieces'],pixels);after=ray_owners(pieces,pixels)
contacts=[];overlaps=[]
for i,a in enumerate(pieces):
 for b in pieces[i+1:]:
  region=Polygon(a['footprint_world']).intersection(Polygon(b['footprint_world']));depth=min(a['top_z'],b['top_z'])-max(a['bottom_z'],b['bottom_z'])
  if region.area<1e-8:continue
  if depth>1e-6:overlaps.append({'a':a['id'],'b':b['id'],'area':region.area,'depth':depth})
  elif abs(depth)<1e-6:contacts.append({'a':a['id'],'b':b['id'],'area':region.area})
rows=[{'pixel':r['pixel'],'expected':e,'before':b,'after':a,'corrected':a==e}for r,e,b,a in zip(missing,expected,before,after)]
report={'status':'PRIVATE_CORRECTED_GEOMETRY_PROPOSAL_NEEDS_SAVED_MODEL_REVIEW','pieces':pieces,'ground_plane_z':min(p['bottom_z']for p in pieces),'baseline_model_sha256':old['model_sha256'],'ownership_sha256':hashlib.sha256((WORK/'loose-planks-ownership-v3/report.json').read_bytes()).hexdigest(),'targets':rows,'source_side_reassignments':reassigned,'source_long_side_polygon':list(side_trace.exterior.coords),'long_end_adjustment_source_pixels':2,'corrected_target_count':sum(r['corrected']for r in rows),'bearing_contacts':contacts,'interpenetrations':overlaps,'remaining':[r for r in rows if not r['corrected']],'limits':['Triangle-ray checks use proposed prism faces, not reopened Blender geometry.','Inherited heights are hypotheses; changed feet require fresh actual receiver queries.','Manual domains are conservative;707 other pixels remain uncertain, not declared foreign.','No texture synthesis or geometry approval.']}
OUT.mkdir();(OUT/'plan.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({'out':str(OUT),'corrected':report['corrected_target_count'],'targets':len(rows),'contacts':len(contacts),'overlaps':overlaps,'remaining':report['remaining']}))
