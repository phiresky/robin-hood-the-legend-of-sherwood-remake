"""Infer fuller rear basal wood while retaining native camera-ray front surfaces."""
import json,math,os,shutil,sys,hashlib
from pathlib import Path
import bpy,bmesh,numpy as np
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(Path(__file__).parent))
from render_slots import acquire
from review_evidence import sha
R=ROOT/'level-editor/work/active/croisement01/restart2';src=R/'tree05-v8';dest=R/'tree05-basal-depth-v8';w=src/'assets/croisement01-tree-05';out=dest/'assets/croisement01-tree-05';assert not dest.exists();acquire(slots=2)
for f in src.iterdir():
 if f.is_file():dest.mkdir(exist_ok=True);shutil.copy2(f,dest/f.name)
out.mkdir(parents=True)
for f in w.iterdir():
 if f.suffix=='.blend' or f.name in ['model.blend','inspection'] or f.name.endswith('.blend1'):continue
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
import restart2_tree05 as helper
from restart2_tree18 import union
helper.DENSE=True
if 'defer_union' in o:del o['defer_union']
roots=[]
for tipx,tipy in [(302,-420),(329,-406),(354,-425)]:
 centers=[Vector((329,-477,99))];radii=[8]
 for t in np.linspace(.04,1,25):
  t=float(t)
  radius=float(np.interp(t,[0,.32,.68,1],[8,5,2,.4]))
  p0=Vector((329,-477,99));p1=Vector((329,-477,65));p2=Vector(((329+tipx)/2,-455,53));p3=Vector((tipx,tipy,53));center=(1-t)**3*p0+3*(1-t)**2*t*p1+3*(1-t)*t*t*p2+t**3*p3;centers.append(center);radii.append(radius)
 root=helper.tube('Inferred rear subsurface root',centers,radii);union(o,root);roots.append(dict(centers=[list(p) for p in centers],radii=radii))
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
_reproject(cfg,out/'projection-new-roots')
bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'));report=dict(status='PRIVATE_REAR_ROOT_HYPOTHESIS_REQUIRES_TOPOLOGY_SOURCE_AND_CONTACT_REVIEW',source_sha256=sha(w/'model.blend'),model_sha256=sha(out/'model.blend'),roots=roots,scope='Three inferred posterior roots extend under the private bank rather than stretching the original source-facing flare. Source ownership unchanged; new unseen root surfaces neutral. Bank still needs separate geometry approval.');(dest/'construction.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report))
import render_candidate;sys.argv=['render','--',str(out)];render_candidate.main()
