"""Inspect actual saved faces and all independently observed source targets."""
import hashlib,json,math,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement/restart2';BASE=WORK/'loose-planks-candidate-v4';OUT=BASE/'saved-source-edge-audit.json'
if OUT.exists():raise FileExistsError(OUT)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
bpy.ops.wm.open_mainfile(filepath=str(BASE/'model.blend'));bpy.context.view_layer.update();objects=[o for o in bpy.context.scene.objects if o.type=='MESH'];vv=[];ff=[];owners=[]
for o in objects:
 offset=len(vv);vv.extend(o.matrix_world@v.co for v in o.data.vertices)
 for p in o.data.polygons:
  ff.append(tuple(offset+i for i in p.vertices));owners.append({'piece':o.data.name,'face':p.index,'role':'top'if p.normal.z>.99 else'bottom'if p.normal.z<-.99 else'side'})
tree=BVHTree.FromPolygons(vv,ff);s=math.sin(math.radians(35));c=math.cos(math.radians(35));back=Vector((0,-c,s));source=json.loads((WORK/'timber-observed-domains-v1/report.json').read_text());rows=[]
for r in source['pixels']:
 x,y=r['pixel'];origin=Vector((x+.5,-(y+.5)/s,0))+back*1000;loc,normal,index,distance=tree.ray_cast(origin,-back);actual=owners[index]if loc is not None else None
 matched=actual is not None and (actual['piece'],actual['role'])==tuple(r['owner']) and normal.dot(back)>=.05
 rows.append({'pixel':[x,y],'expected':r['owner'],'actual':actual,'cosine':normal.dot(back)if normal else None,'matched':matched})
edge_recoveries=[]
for row in rows:
 if row['matched']:continue
 x,y=row['pixel'];found=None
 for eps in(.001,.01):
  for dx,dy in[(eps,0),(-eps,0),(0,eps),(0,-eps),(eps,eps),(-eps,-eps),(eps,-eps),(-eps,eps)]:
   origin=Vector((x+.5+dx,-(y+.5+dy)/s,0))+back*1000;loc,normal,index,distance=tree.ray_cast(origin,-back)
   actual=owners[index]if loc is not None else None
   if actual and(actual['piece'],actual['role'])==tuple(row['expected']) and normal.dot(back)>=.05:
    found={'pixel':row['pixel'],'expected':row['expected'],'offset':[dx,dy]};break
  if found:break
 if found:edge_recoveries.append(found)
report={'strict_matches':sum(r['matched']for r in rows),'tiny_edge_recoveries':edge_recoveries,'remaining':[r for r in rows if not r['matched']and r['pixel']not in[q['pixel']for q in edge_recoveries]],'model_sha256':hashlib.sha256((BASE/'model.blend').read_bytes()).hexdigest(),'limitation':'Offsets are source-pixel fractions, not geometry edits. Recovery tests only; no new accepted texture or approval.'};OUT.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({'strict':report['strict_matches'],'edge_recoveries':len(edge_recoveries),'remaining':len(report['remaining'])}))
