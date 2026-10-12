"""Infer fuller rear basal wood while retaining native camera-ray front surfaces."""
import json,math,os,shutil,sys,hashlib
from pathlib import Path
import bpy,bmesh,numpy as np
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'));sys.path.insert(0,str(Path(__file__).parent))
from render_slots import acquire
from review_evidence import sha
R=ROOT/'level-editor/work/active/croisement01/restart2';src=R/'tree05-basal-depth-v10';dest=R/'tree05-basal-depth-v12';w=src/'assets/croisement01-tree-05';out=dest/'assets/croisement01-tree-05';assert not dest.exists();acquire(slots=2)
for f in src.iterdir():
 if f.is_file():dest.mkdir(exist_ok=True);shutil.copy2(f,dest/f.name)
out.mkdir(parents=True)
for f in w.iterdir():
 if f.suffix=='.blend' or f.name in ['model.blend','inspection'] or f.name.startswith('projection') or f.name.endswith('.blend1'):continue
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
# Pin all vertices of first-hit native wood faces; fair inferred rear junctions only.
from PIL import Image
points=np.asarray([list(o.matrix_world@v.co) for v in o.data.vertices]);faces=[tuple(f.vertices) for f in o.data.polygons];tree=BVHTree.FromPolygons([Vector(p) for p in points],faces);mask=np.asarray(Image.open(src/'wood-domain.png'))>0;sn,cs=math.sin(math.radians(35)),math.cos(math.radians(35));ray=Vector((0,-cs,sn));pinned=set();rays=[]
for y,x in zip(*np.nonzero(mask)):
 origin=Vector((265+float(x)+.5,-(float(y)+.5)/sn,0))+ray*5000;hit,_,face,_=tree.ray_cast(origin,-ray,10000)
 if hit is not None:pinned.update(faces[face]);rays.append((origin,face))
edges=np.asarray([list(e.vertices) for e in o.data.edges]);
for _ in range(2):
 near=np.isin(edges[:,0],list(pinned))|np.isin(edges[:,1],list(pinned));pinned.update(edges[near].flatten().tolist())
active=points[:,2]<140;active[list(pinned)]=False;degree=np.bincount(edges.flatten(),minlength=len(points));original_points=points.copy()
for iteration in range(40):
 for factor in [.5,-.51]:
  totals=np.zeros_like(points);np.add.at(totals,edges[:,0],points[edges[:,1]]);np.add.at(totals,edges[:,1],points[edges[:,0]]);delta=totals/degree[:,None]-points;points[active]+=factor*delta[active]
assert np.array_equal(points[list(pinned)],original_points[list(pinned)]);after_tree=BVHTree.FromPolygons([Vector(p) for p in points],faces);changed_hits=0
for origin,face in rays:
 hit,_,new_face,_=after_tree.ray_cast(origin,-ray,10000)
 if hit is None or new_face!=face:changed_hits+=1
assert changed_hits==0,changed_hits
inverse=o.matrix_world.inverted()
for vertex,p in zip(o.data.vertices,points):vertex.co=inverse@Vector(p)
o.data.update();roots=dict(method='Taubin fairing of inferred rear basal mesh; all vertices of native first-hit faces pinned, all source first-hit face identities exact.',pinned_vertices=len(pinned),active_vertices=int(active.sum()),changed_native_first_hits=changed_hits,maximum_displacement=float(np.linalg.norm(points-original_points,axis=1).max()))
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
_reproject(cfg,out/'projection-rear-fair')
bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'));report=dict(status='PRIVATE_REAR_ROOT_HYPOTHESIS_REQUIRES_TOPOLOGY_SOURCE_AND_CONTACT_REVIEW',source_sha256=sha(w/'model.blend'),model_sha256=sha(out/'model.blend'),roots=roots,scope='Three inferred posterior roots extend under the private bank; the original basal depth interval is fixed. Source ownership unchanged; new unseen root surfaces neutral. Bank still needs separate geometry approval.');(dest/'construction.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report))
import render_candidate;sys.argv=['render','--',str(out)];render_candidate.main()
