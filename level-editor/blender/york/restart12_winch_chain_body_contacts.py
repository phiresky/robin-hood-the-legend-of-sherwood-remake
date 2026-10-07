"""Locate chain contact candidates against the retained solid body and room."""
import hashlib,json,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/york-refinement/restart2/winch-complete-chain-motion-v1';OUT=BASE/'body-contact-candidates.json'
if OUT.exists():raise FileExistsError(OUT)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
from mathutils.bvhtree import BVHTree
bpy.ops.wm.open_mainfile(filepath=str(BASE/'model.blend'));scene=bpy.context.scene;motion=json.loads((BASE/'motion.json').read_text());links=[o for o in scene.objects if o.name.startswith('Complete chain loop link')];receivers=[o for o in scene.objects if o.type=='MESH' and o not in links and not o.name.startswith('Inferred upper')];rows=[]
def tree(objects):
 vertices=[];faces=[];owners=[]
 for o in objects:
  offset=len(vertices);vertices.extend(o.matrix_world@v.co for v in o.data.vertices)
  o.data.calc_loop_triangles()
  for f in o.data.loop_triangles:faces.append(tuple(offset+i for i in f.vertices));owners.append(o.name)
 return BVHTree.FromPolygons(vertices,faces,all_triangles=True),owners
for r in motion['rows']:
 scene.frame_set(r['tick']);bpy.context.view_layer.update();body,owners=tree(receivers);chain,_=tree(links);counts={}
 for a,b in chain.overlap(body):counts[owners[b]]=counts.get(owners[b],0)+1
 rows.append({'frame':r['source_frame'],'triangle_overlap_candidates':counts})
OUT.write_text(json.dumps({'status':'Diagnostic triangle intersection candidates; distinguish intentional drive engagement from unresolved solid penetration before accepting','model_sha256':hashlib.sha256((BASE/'model.blend').read_bytes()).hexdigest(),'frames':rows},indent=2)+'\n');print(json.dumps(rows,indent=2))
