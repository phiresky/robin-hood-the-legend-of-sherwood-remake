"""Private smooth bank joint with exact original top boundaries retained."""
import json,sys,hashlib,shutil
from pathlib import Path
import bpy,bmesh,numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
from review_evidence import sha
R=ROOT/'level-editor/work/croisement01-refinement/restart2';source=R/'tree05-v6/assets/croisement01-tree-05';dest=R/'tree05-soil-joint-v2';worker=dest/'assets/croisement01-tree-05';assert not dest.exists();acquire();bpy.ops.wm.open_mainfile(filepath=str(source/'model.blend'));bpy.context.preferences.filepaths.save_version=0;cfg=json.loads((source/'workspace.json').read_text());collection=bpy.data.collections[cfg['collection_name']];before=sha(source/'model.blend')
def geometry(o):return hashlib.sha256(json.dumps([[list(v.co) for v in o.data.vertices],[list(f.vertices) for f in o.data.polygons]],separators=(',',':')).encode()).hexdigest()
def closed_mesh(name,verts,faces):
 mesh=bpy.data.meshes.new(name);mesh.from_pydata(verts,[],faces);mesh.update();bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));assert all(e.is_manifold for e in bm.edges),name;bm.to_mesh(mesh);bm.free();return mesh
protected={o.get('source_node'):geometry(o) for o in collection.all_objects if o.type=='MESH'};banks=[o for o in collection.all_objects if o.type=='MESH' and o.get('source_node') in {'building-006','building-007'}];vertices=[];faces=[]
for o in banks:
 off=len(vertices);vertices.extend(o.matrix_world@v.co for v in o.data.vertices);faces.extend(tuple(off+i for i in f.vertices) for f in o.data.polygons)
terrain=BVHTree.FromPolygons(vertices,faces);xs=np.linspace(249,409,41);ys=np.linspace(-565,-389,45);ground=np.zeros((len(xs),len(ys)))
for i,x in enumerate(xs):
 for j,y in enumerate(ys):
  hit,_,_,_=terrain.ray_cast(Vector((x,y,2000)),Vector((0,0,-1)),4000);ground[i,j]=hit.z if hit is not None else 0
z=ground.copy();core=((xs[:,None]-329)/32)**2+((ys[None,:]+477)/18)**2<=1;fixed=np.zeros_like(core);fixed[[0,-1],:]=True;fixed[:,[0,-1]]=True;fixed|=core;z[core]=80.5
for it in range(5000):
 proposed=z.copy();proposed[1:-1,1:-1]=(z[:-2,1:-1]+z[2:,1:-1]+z[1:-1,:-2]+z[1:-1,2:])/4;proposed[fixed]=z[fixed];delta=np.max(np.abs(proposed-z));z=proposed
 if delta<1e-6:break
shells=[]
for bank in banks:
 verts=[];idx={};top=[]
 for face in bank.data.polygons:
  if face.normal.z<=.1:continue
  points=[bank.matrix_world@bank.data.vertices[k].co for k in face.vertices]
  if max(p.z for p in points)<.01:continue
  row=[]
  for p in points:
   key=tuple(round(float(v),5) for v in p)
   if key not in idx:idx[key]=len(verts);verts.append(tuple(p))
   row.append(idx[key])
  top.append(tuple(row))
 assert top;count={};n=len(verts);allfaces=top+[tuple(k+n for k in reversed(f)) for f in top]
 for f in top:
  for a,b in zip(f,f[1:]+f[:1]):
   key=tuple(sorted((a,b)));count[key]=(count.get(key,(0,None))[0]+1,(a,b))
 for uses,(a,b) in count.values():
  if uses==1:allfaces.append((a,a+n,b+n,b))
 verts += [(x,y,0) for x,y,z0 in verts];mesh=closed_mesh('Closed native bank top shell',verts,allfaces)
 for mat in bank.data.materials:mesh.materials.append(mat)
 # Imported bank coordinates are world positions; keep object transform exact.
 for vertex in mesh.vertices:vertex.co=bank.matrix_world.inverted()@vertex.co
 bank.data=mesh;shells.append(dict(node=bank['source_node'],top_faces=len(top),top_vertices=n,method='Original upward top coordinates retained; side and bottom shell closed to zero before local cut.'))
bpy.ops.mesh.primitive_cube_add(size=1,location=(329,-477,500));cutter=bpy.context.object;cutter.dimensions=(160,176,1020);bpy.ops.object.transform_apply(location=False,rotation=False,scale=True)
for bank in banks:
 bpy.context.view_layer.objects.active=bank;mod=bank.modifiers.new('Bounded local joint replacement','BOOLEAN');mod.operation='DIFFERENCE';mod.solver='EXACT';mod.object=cutter;bpy.ops.object.modifier_apply(modifier=mod.name)
bpy.data.objects.remove(cutter,do_unlink=True)
verts=[(float(x),float(y),float(z[i,j])) for i,x in enumerate(xs) for j,y in enumerate(ys)];n=len(verts);ny=len(ys);verts += [(x,y,0) for x,y,zz in verts];faces=[]
for i in range(len(xs)-1):
 for j in range(ny-1):
  a=i*ny+j;b=a+ny;faces += [(a,b,b+1,a+1),(n+a,n+a+1,n+b+1,n+b)]
boundary=list(range(ny))+[i*ny+ny-1 for i in range(1,len(xs))]+list(range((len(xs)-1)*ny+ny-2,(len(xs)-1)*ny-1,-1))+[i*ny for i in range(len(xs)-2,0,-1)]
for a,b in zip(boundary,boundary[1:]+boundary[:1]):faces.append((a,b,b+n,a+n))
mesh=closed_mesh('Continuous inferred bank joint',verts,faces);obj=bpy.data.objects.new(mesh.name,mesh);collection.objects.link(obj);obj['source_node']='tree05-local-soil-joint';obj['asset_group']='croisement01-tree05-local-soil';mat=bpy.data.materials.new('Unapproved inferred soil');mat.diffuse_color=(.32,.27,.17,1);mesh.materials.append(mat)
for face in mesh.polygons:face.use_smooth=True
assert all(geometry(o)==protected[o.get('source_node')] for o in collection.all_objects if o.type=='MESH' and o is not obj and o not in banks)
(worker/'modified').mkdir(parents=True);(worker/'inspection').mkdir();shutil.copy2(source/'modified/views.json',worker/'modified/views.json');(worker/'workspace.json').write_text(json.dumps(cfg,indent=2)+'\n');bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'));assert sha(source/'model.blend')==before
(dest/'construction.json').write_text(json.dumps(dict(status='Private joint geometry hypothesis, not approved',source_sha256=before,model_sha256=sha(worker/'model.blend'),wood_and_other_context_unchanged=True,terrain_changed=['building-006','building-007','tree05-local-soil-joint'],shell_repair=shells,bounds=[[249,409],[-565,-389]],inferred='Smooth harmonic soil shoulder to exact original perimeter heights; source-backed tree flare fixed. Hidden terrain is inferred, gameplay remains untouched.',iterations=it+1,convergence=float(delta),max_raise=float(np.max(z-ground)),max_lower=float(np.max(ground-z))),indent=2)+'\n');print(worker)
