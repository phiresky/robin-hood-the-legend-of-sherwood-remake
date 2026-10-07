"""Read-only evidence for local root/bank conflicts before a joint revision."""
import argparse,json,sys,math
from pathlib import Path
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
from review_evidence import sha
p=argparse.ArgumentParser();p.add_argument('tree',type=int);p.add_argument('revision',type=int);a=p.parse_args(sys.argv[sys.argv.index('--')+1:]);r=ROOT/'level-editor/work/croisement01-refinement/restart2';w=r/f'tree{a.tree:02d}-v{a.revision}'/f'assets/croisement01-tree-{a.tree:02d}';out=w/'inspection/bank-joint-probe.json';assert not out.exists();acquire();bpy.ops.wm.open_mainfile(filepath=str(w/'model.blend'));s,c=math.sin(math.radians(35)),math.cos(math.radians(35));ray=Vector((0,-c,s));banks=[];trees=[]
for obj in bpy.data.objects:
 node=obj.get('source_node','')
 if obj.type!='MESH' or node not in {'building-003','building-004','building-006','building-007'}:continue
 pts=[obj.matrix_world@v.co for v in obj.data.vertices];faces=[tuple(f.vertices) for f in obj.data.polygons];tree=BVHTree.FromPolygons(pts,faces);trees.append((node,tree));unique={tuple(round(float(x),4) for x in p) for p in pts};banks.append(dict(node=node,points=sorted(unique),faces=len(faces),bounds=[[min(p[i] for p in pts),max(p[i] for p in pts)] for i in range(3)]))
xy=[(x,y) for x in ([195,215,235] if a.tree==4 else [295,325,355]) for y in ([565,580,589,600] if a.tree==4 else [185,200,215,225])];casts=[]
for x,y in xy:
 point=Vector((x,-y/s,0));hits=[]
 for node,tree in trees:
  start=point+ray*5000
  for i in range(20):
   hit,normal,face,distance=tree.ray_cast(start,-ray,10000)
   if hit is None:break
   hits.append(dict(node=node,point=list(hit),normal=list(normal),depth=hit.dot(ray)));start=hit-ray*.02
 casts.append(dict(source_pixel=[x,y],intersections=sorted(hits,key=lambda h:-h['depth'])))
out.write_text(json.dumps(dict(model_sha256=sha(w/'model.blend'),banks=banks,source_rays=casts,scope='Unmodified archived terrain; evidence for a bounded future joint, no ready claim.'),indent=2)+'\n');print(out)
