"""Read-only saved leaf-plane facing and visibility witnesses before sampling retry."""
import json,sys,hashlib
from pathlib import Path
import bpy
from mathutils import Matrix,Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
R=ROOT/'level-editor/work/croisement01-refinement/restart2';case=R/'approved-tree02-fill-v1/croisement01-tree-02';model=case/'baked-v1-luminance/worker.blend';out=case/'baked-v1-luminance/exact-crown-facing-probe.json';assert not out.exists();acquire();digest=hashlib.sha256(model.read_bytes()).hexdigest();bpy.ops.wm.open_mainfile(filepath=str(model));frames=json.loads((case/'experiment/views.json').read_text());o=bpy.data.objects['Tree02 inferred crown'];vertices=[];faces=[];offsets={}
for name in frames['object_names']:
 obj=bpy.data.objects[name];offsets[name]=len(faces);base=len(vertices);vertices.extend(obj.matrix_world@v.co for v in obj.data.vertices);faces.extend(tuple(base+i for i in p.vertices) for p in obj.data.polygons)
tree=BVHTree.FromPolygons(vertices,faces);cameras=[(v,Matrix(v['camera_matrix_world']).inverted(),Matrix(v['camera_matrix_world']).to_3x3()@Vector((0,0,1))) for v in frames['views']];no_one=[];no_two=[];witness=[]
for face in o.data.polygons:
 normal=(o.matrix_world.to_3x3().inverted().transposed()@face.normal).normalized();dots=[normal.dot(d) for _,_,d in cameras]
 if max(dots)<=.12:no_one.append(face.index)
 if max(map(abs,dots))<=.12:no_two.append(face.index)
 for (view,inverse,direction),dot in zip(cameras,dots):
  if dot>=-.12:continue
  center=o.matrix_world@face.center;local=inverse@center
  if max(abs(local.x),abs(local.y))>view['ortho_scale']/2:continue
  hit=tree.ray_cast(center+direction*2000,-direction,4000)
  if hit[2]==offsets[o.name]+face.index:
   witness.append(dict(face=face.index,view=view['index'],facing=dot,hit_distance=(hit[0]-center).length));break
assert len(no_one)==350 and not no_two and witness
assert hashlib.sha256(model.read_bytes()).hexdigest()==digest
out.write_text(json.dumps(dict(status='PASS_EXACT_SAVED_CROWN_DIAGNOSTIC',model_sha256=digest,faces=len(o.data.polygons),no_one_sided_camera=no_one,no_two_sided_camera=no_two,visible_back_face_witnesses=witness,scope='Only two-sided thin crown triangles; no closed wood sampling change. Geometry and materials unchanged.'),indent=2)+'\n');print('EXACT CROWN PROBE PASS',len(no_one),len(witness),flush=True)
