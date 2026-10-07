"""Test source-preserving frame depth inference without changing saved candidates."""
import ast,json,math,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement/restart2';BASE=WORK/'winch-components-source-v2';OUT=WORK/'winch-frame-depth-study-v1'
if OUT.exists():raise FileExistsError(OUT)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy,numpy as np
from mathutils import Vector,Matrix
from mathutils.bvhtree import BVHTree
s,c=math.sin(math.radians(35)),math.cos(math.radians(35));up=Vector((0,0,1));screen_side=Vector((1,0,0));spacing=3.5/c;back=Vector((0,-c,s))
recipe=Path(__file__).with_name('restart13_winch_return_study.py');defs=[n for n in ast.parse(recipe.read_text()).body if isinstance(n,ast.FunctionDef)];exec(compile(ast.Module(body=defs,type_ignores=[]),str(recipe),'exec'));center=world(2410,1064,104);axis=(center-world(2402,1050,104)).normalized();radial=Vector((-axis.y,axis.x,0))
bpy.ops.wm.open_mainfile(filepath=str(BASE/'model.blend'));scene=bpy.context.scene;scene.frame_set(88);bpy.context.view_layer.update();scope=set(json.loads((BASE/'component-freeze.json').read_text())['scope']);front={'Angled left frame brace.001','Angled right frame brace.001','Frame top saddle.001','Frame foot rail.001'};floor=90.00101/c;height=113/c-floor
pose,params=route(9.4,2399.5);local=[];surface=[];rx=1.2;perimeter=2*math.pi*rx+4*(3-rx)
for j in range(32):
 p,n=capsule(j*perimeter/32,rx)
 for k in range(8):phi=k*math.tau/8;local.append(p+n*(.4*math.cos(phi))+Vector((0,0,.4*math.sin(phi))))
for j in range(32):
 for k in range(8):surface.append((j*8+k,((j+1)%32)*8+k,((j+1)%32)*8+(k+1)%8,j*8+(k+1)%8))
vv=[];ff=[]
for i in range(72):
 p,r=pose(i*spacing+5.5/c,i);off=len(vv);vv.extend(p+r@v for v in local);ff.extend(tuple(off+j for j in f) for f in surface)
chain=BVHTree.FromPolygons(vv,ff);rows=[]
for shift in (-12,-8,-4,0,4,8,12,16):
 vertices=[];faces=[];owners=[];errors=[];floor_min=[]
 for o in scene.objects:
  if o.name not in scope:continue
  off=len(vertices)
  for vertex in o.data.vertices:
   p=o.matrix_world@vertex.co;q=p+back*(shift*(p.z-floor)/height) if o.name in front else p;vertices.append(q);errors.append(abs((-q.y*s-q.z*c)-(-p.y*s-p.z*c)))
   if o.name in front:floor_min.append(q.z*c)
  o.data.calc_loop_triangles()
  for f in o.data.loop_triangles:faces.append(tuple(off+i for i in f.vertices));owners.append(o.name)
 body=BVHTree.FromPolygons(vertices,faces,all_triangles=True);overlaps=chain.overlap(body);counts={}
 for _,f in overlaps:counts[owners[f]]=counts.get(owners[f],0)+1
 rows.append({'frame_top_view_ray_shift_world':shift,'maximum_native_projection_error':max(errors),'minimum_front_frame_game_z':min(floor_min),'body_intersection_pairs':counts,'pair_count':len(overlaps)})
OUT.mkdir();(OUT/'study.json').write_text(json.dumps({'status':'Depth-only diagnostic, no geometry saved or approval inferred','changed_components':sorted(front),'chain_path':params,'rows':rows},indent=2)+'\n');print(json.dumps(rows,indent=2))
