"""Private bounded soil support hypothesis for the source-observed root flare."""
import json,math,sys,hashlib,shutil
from pathlib import Path
import bpy,bmesh
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
from review_evidence import sha
R=ROOT/'level-editor/work/croisement01-refinement/restart2';source=R/'tree05-v6/assets/croisement01-tree-05';dest=R/'tree05-soil-joint-v1';worker=dest/'assets/croisement01-tree-05'
assert not dest.exists();assert (source/'inspection/actual-materials/sheet.png').exists();acquire();bpy.ops.wm.open_mainfile(filepath=str(source/'model.blend'));bpy.context.preferences.filepaths.save_version=0;before=sha(source/'model.blend');cfg=json.loads((source/'workspace.json').read_text());collection=bpy.data.collections[cfg['collection_name']]
def geometry(o):return hashlib.sha256(json.dumps([[list(v.co) for v in o.data.vertices],[list(f.vertices) for f in o.data.polygons]],separators=(',',':')).encode()).hexdigest()
protected={o.get('source_node'):geometry(o) for o in collection.all_objects if o.type=='MESH'};vertices=[];faces=[]
for o in collection.all_objects:
 if o.type!='MESH' or o.get('source_node') not in {'building-006','building-007','ground'}:continue
 offset=len(vertices);vertices.extend(o.matrix_world@v.co for v in o.data.vertices);faces.extend(tuple(offset+i for i in f.vertices) for f in o.data.polygons)
terrain=BVHTree.FromPolygons(vertices,faces);xs=[249+4*i for i in range(41)];ys=[-565+4*i for i in range(45)];verts=[];height=[];ground=[]
for x in xs:
 for y in ys:
  hit,normal,_,_=terrain.ray_cast(Vector((x,y,2000)),Vector((0,0,-1)),4000)
  if hit is None:raise ValueError('Missing local terrain witness')
  base=hit.z;radius=math.sqrt(((x-329)/70)**2+((y+477)/80)**2);u=min(1,max(0,(radius-.35)/.65));weight=1-u*u*(3-2*u);top=max(base,base+(80.5-base)*weight);verts.append((x,y,top));height.append(top);ground.append(base)
ny=len(ys);n=len(verts);faces=[];active=[]
for i in range(len(xs)-1):
 for j in range(ny-1):
  a=i*ny+j;cell=[a,a+ny,a+ny+1,a+1]
  if max(height[k]-ground[k] for k in cell)>.1:active.append(cell)
verts += [(x,y,z-2) for (x,y,_),z in zip(verts,ground)];edge_counts={}
for cell in active:
 faces.append(tuple(cell));faces.append(tuple(k+n for k in reversed(cell)))
 for a,b in zip(cell,cell[1:]+cell[:1]):
  key=tuple(sorted((a,b)));edge_counts[key]=(edge_counts.get(key,(0,None))[0]+1,(a,b))
for count,(a,b) in edge_counts.values():
 if count==1:faces.append((a,a+n,b+n,b))
mesh=bpy.data.meshes.new('Inferred local root soil shoulder');mesh.from_pydata(verts,[],faces);mesh.update();bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));assert all(e.is_manifold for e in bm.edges);bm.to_mesh(mesh);bm.free();obj=bpy.data.objects.new(mesh.name,mesh);collection.objects.link(obj);obj['source_node']='tree05-local-soil-joint';obj['asset_group']='croisement01-tree05-local-soil';obj['review_scope']='Inferred local soil geometry only; native ownership and appearance separate';mat=bpy.data.materials.new('Unapproved inferred root soil');mat.diffuse_color=(.32,.27,.17,1);mesh.materials.append(mat)
for face in mesh.polygons:face.use_smooth=True
assert all(geometry(o)==protected[o.get('source_node')] for o in collection.all_objects if o.type=='MESH' and o is not obj)
(worker/'modified').mkdir(parents=True);(worker/'inspection').mkdir();shutil.copy2(source/'modified/views.json',worker/'modified/views.json');(worker/'workspace.json').write_text(json.dumps(cfg,indent=2)+'\n');bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'));assert sha(source/'model.blend')==before
(dest/'construction.json').write_text(json.dumps(dict(status='Private joint hypothesis; not approved',source_model_sha256=before,model_sha256=sha(worker/'model.blend'),wood_and_all_existing_geometry_unchanged=True,protected_geometry=protected,new_node=obj['source_node'],source_constraint='Observed flare is above lower bank006 and in front of upper bank007. A continuous local soil shoulder can support it without flattening wood.',inferred='Hidden soil slope, elliptical70x80-unit taper around(329,-477), upper seat80.5. Existing bank006/007 unchanged; final integration must reconcile overlap/source ownership.',maximum_soil_raise=max(a-b for a,b in zip(height,ground)),active_cells=len(active)),indent=2)+'\n');print(worker)
