"""Infer fuller rear basal wood while retaining native camera-ray front surfaces."""
import json,math,os,shutil,sys,hashlib
from pathlib import Path
import bpy,bmesh,numpy as np
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'));sys.path.insert(0,str(Path(__file__).parent))
from render_slots import acquire
from review_evidence import sha
R=ROOT/'level-editor/work/croisement01-refinement/restart2';src=R/'tree05-basal-depth-v2';dest=R/'tree05-basal-depth-v10';w=src/'assets/croisement01-tree-05';out=dest/'assets/croisement01-tree-05';assert not dest.exists();acquire(slots=2)
for f in src.iterdir():
 if f.is_file():dest.mkdir(exist_ok=True);shutil.copy2(f,dest/f.name)
out.mkdir(parents=True)
for f in w.iterdir():
 if f.name in ['model.blend','inspection'] or f.name.startswith('projection') or f.name.endswith('.blend1'):continue
 if f.is_dir():shutil.copytree(f,out/f.name,copy_function=os.link)
 else:shutil.copy2(f,out/f.name)
cfg=json.loads((out/'workspace.json').read_text());cfg=json.loads(json.dumps(cfg).replace(str(w),str(out)));(out/'workspace.json').write_text(json.dumps(cfg,indent=2)+'\n');(out/'inspection').mkdir()
bpy.ops.wm.open_mainfile(filepath=str(w/'model.blend'));bpy.context.preferences.filepaths.save_version=0
o=next(o for o in bpy.context.scene.objects if o.get('source_node')=='scenery-tree05-wood')
from mathutils.bvhtree import BVHTree
with bpy.data.libraries.load(str(R/'tree05-regional-banks-v15/assets/croisement01-tree-05/model.blend'),link=False) as (a,b):b.objects=a.objects
for linked in b.objects:
 if linked is not None:bpy.context.scene.collection.objects.link(linked)
bpy.context.view_layer.update();vertices=[];faces=[]
for context in b.objects:
 if context is None or context.type!='MESH' or context.get('source_node') not in [f'building-{i:03d}' for i in range(10)]:continue
 off=len(vertices);vertices.extend(context.matrix_world@v.co for v in context.data.vertices);faces.extend(tuple(off+k for k in f.vertices) for f in context.data.polygons)
soil=BVHTree.FromPolygons(vertices,faces)
collection=bpy.data.collections[cfg['collection_name']]
for context in b.objects:
 if context is not None and context.type=='MESH' and context.get('source_node') in ['building-000',*[f'building-{i:03d}' for i in range(2,8)]]:
  target=next(x for x in collection.all_objects if x.get('source_node')==context.get('source_node'));assert target.matrix_world==context.matrix_world;target.data=context.data.copy()
for context in b.objects:
 if context is not None:bpy.data.objects.remove(context,do_unlink=True)
# Seat only the new posterior root lengths. The connected v2 junction stays fixed.
original=R/'tree05-v8/assets/croisement01-tree-05/model.blend'
with bpy.data.libraries.load(str(original),link=False) as (a,b):b.objects=a.objects
for linked in b.objects:
 if linked is not None:bpy.context.scene.collection.objects.link(linked)
bpy.context.view_layer.update();old=next(x for x in b.objects if x is not None and x.get('source_node')=='scenery-tree05-wood');original_points=[old.matrix_world@v.co for v in old.data.vertices];cut=max(p.y for p in original_points if p.z<130)+.25
for context in b.objects:
 if context is not None:bpy.data.objects.remove(context,do_unlink=True)
changed=0;maximum=0;inverse=o.matrix_world.inverted()
for vertex in o.data.vertices:
 p=o.matrix_world@vertex.co
 if p.z>=130 or p.y<=cut:continue
 t=min(1,max(0,(p.y-cut)/18));weight=t*t*(3-2*t);hit,_,_,_=soil.ray_cast(Vector((p.x,p.y,2000)),Vector((0,0,-1)),4000);assert hit is not None
 center_z=99-(p.y+477)*.6;drop=max(0,center_z-(hit.z-5))*weight
 if drop:vertex.co=inverse@(p-Vector((0,0,drop)));changed+=1;maximum=max(maximum,drop)
o.data.update();roots=dict(original_basal_max_y=cut-.25,changed_vertices=changed,maximum_lower=maximum,method='Smoothly seat only posterior added root lengths past every original basal vertex. Existing root junction stays connected.')
bm=bmesh.new();bm.from_mesh(o.data);todo=set(bm.verts);components=[]
while todo:
 stack=[todo.pop()];n=0
 while stack:
  v=stack.pop();n+=1
  for edge in v.link_edges:
   other=edge.other_vert(v)
   if other in todo:todo.remove(other);stack.append(other)
 components.append(n)
assert len(components)==1,components
assert all(e.is_manifold for e in bm.edges);bm.free()
bpy.ops.wm.save_as_mainfile(filepath=str(out/'geometry-before-projection.blend'))
from refinement_workspace import _reproject
_reproject(cfg,out/'projection-rear-seat')
bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'));report=dict(status='PRIVATE_REAR_ROOT_HYPOTHESIS_REQUIRES_TOPOLOGY_SOURCE_AND_CONTACT_REVIEW',source_sha256=sha(w/'model.blend'),model_sha256=sha(out/'model.blend'),roots=roots,scope='Three inferred posterior roots extend under the private bank; the original basal depth interval is fixed. Source ownership unchanged; new unseen root surfaces neutral. Bank still needs separate geometry approval.');(dest/'construction.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report))
import render_candidate;sys.argv=['render','--',str(out)];render_candidate.main()
