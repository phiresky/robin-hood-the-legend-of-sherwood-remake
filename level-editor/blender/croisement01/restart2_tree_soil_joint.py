"""Private smooth bank joint with exact original top boundaries retained."""
import argparse,json,sys,hashlib,shutil,math
from PIL import Image
from pathlib import Path
import bpy,bmesh,numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
from review_evidence import sha
parser=argparse.ArgumentParser();parser.add_argument('--tree',type=int,choices=[4,5],required=True);parser.add_argument('--source-revision',type=int,required=True);parser.add_argument('--revision',type=int,required=True);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:]);tree_id=args.tree;profile={4:dict(nodes=['building-003','building-004'],x=[155,275],y=[-1215,-1045]),5:dict(nodes=['building-006','building-007'],x=[249,409],y=[-565,-389])}[tree_id];R=ROOT/'level-editor/work/croisement01-refinement/restart2';source=R/f'tree{tree_id:02d}-v{args.source_revision}/assets/croisement01-tree-{tree_id:02d}';dest=R/f'tree{tree_id:02d}-soil-joint-v{args.revision}';worker=dest/f'assets/croisement01-tree-{tree_id:02d}';assert not dest.exists();acquire();bpy.ops.wm.open_mainfile(filepath=str(source/'model.blend'));bpy.context.preferences.filepaths.save_version=0;cfg=json.loads((source/'workspace.json').read_text());collection=bpy.data.collections[cfg['collection_name']];before=sha(source/'model.blend')
def geometry(o):return hashlib.sha256(json.dumps([[list(v.co) for v in o.data.vertices],[list(f.vertices) for f in o.data.polygons]],separators=(',',':')).encode()).hexdigest()
def closed_mesh(name,verts,faces):
 mesh=bpy.data.meshes.new(name);mesh.from_pydata(verts,[],faces);mesh.update();bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bad=[e for e in bm.edges if not e.is_manifold]
 if bad:print(json.dumps(dict(mesh=name,edges=[dict(points=[list(v.co) for v in e.verts],faces=len(e.link_faces)) for e in bad[:20]])));raise ValueError('Nonmanifold joint shell')
 bm.to_mesh(mesh);bm.free();return mesh
protected={o.get('source_node'):geometry(o) for o in collection.all_objects if o.type=='MESH'};banks=[o for o in collection.all_objects if o.type=='MESH' and o.get('source_node') in set(profile['nodes'])];vertices=[];faces=[]
for o in banks:
 off=len(vertices);vertices.extend(o.matrix_world@v.co for v in o.data.vertices);faces.extend(tuple(off+i for i in f.vertices) for f in o.data.polygons)
terrain=BVHTree.FromPolygons(vertices,faces);xs=np.linspace(*profile['x'],int((profile['x'][1]-profile['x'][0])/2)+1);ys=np.linspace(*profile['y'],int((profile['y'][1]-profile['y'][0])/2)+1);ground=np.zeros((len(xs),len(ys)))
for i,x in enumerate(xs):
 for j,y in enumerate(ys):
  hit,_,_,_=terrain.ray_cast(Vector((x,y,2000)),Vector((0,0,-1)),4000);ground[i,j]=hit.z if hit is not None else 0
wood=next(o for o in collection.all_objects if o.get('source_node')==f'scenery-tree{tree_id:02d}-wood');wood_tree=BVHTree.FromPolygons([wood.matrix_world@v.co for v in wood.data.vertices],[tuple(f.vertices) for f in wood.data.polygons]);wood_ceiling=min((wood.matrix_world@v.co).z for v in wood.data.vertices)+60;z=ground.copy();core=np.zeros_like(z,dtype=bool)
for i,x in enumerate(xs):
 for j,y in enumerate(ys):
  hit,_,_,_=wood_tree.ray_cast(Vector((x,y,-500)),Vector((0,0,1)),2000)
  if hit is not None and hit.z<wood_ceiling:core[i,j]=True;z[i,j]=hit.z-.3
assert core.sum()>20;fixed=np.zeros_like(core);fixed[[0,-1],:]=True;fixed[:,[0,-1]]=True;fixed|=core
# Known native wood rays constrain the soil in front of the tree. This is
# geometric visibility evidence, independent of source color or generated art.
row=next(r for r in json.loads((R.parent/'baseline/masks/manifest.json').read_text())['masks'] if r['index']==tree_id);left,top=row['box_top_left'];domain=np.asarray(Image.open(source.parents[1]/'wood-domain.png'))>0;source_caps=np.full_like(z,np.inf);sine,cosine=math.sin(math.radians(35)),math.cos(math.radians(35));direction=Vector((0,-cosine,sine));constraints=0
for py,px in zip(*np.nonzero(domain)):
 x=left+px+.5
 if x<xs[0] or x>xs[-1]:continue
 origin=Vector((x,-(top+py+.5)/sine,0))+direction*5000;hit,_,_,_=wood_tree.ray_cast(origin,-direction,20000)
 if hit is None:continue
 i=min(len(xs)-2,max(0,int(np.searchsorted(xs,x)-1)));limits=hit.z+(hit.y-ys)*sine/cosine-.5;limits=np.where(ys<=hit.y+2,limits,np.inf)
 source_caps[i]=np.minimum(source_caps[i],limits);source_caps[i+1]=np.minimum(source_caps[i+1],limits);constraints+=1
boundary=np.zeros_like(fixed);boundary[[0,-1],:]=True;boundary[:,[0,-1]]=True
assert not np.any(source_caps[boundary]<ground[boundary]-.01),'Visibility constraint extends beyond local terrain boundary'
z=np.minimum(z,source_caps)
for it in range(5000):
 proposed=z.copy();proposed[1:-1,1:-1]=(z[:-2,1:-1]+z[2:,1:-1]+z[1:-1,:-2]+z[1:-1,2:])/4;proposed[fixed]=z[fixed];proposed=np.minimum(proposed,source_caps);delta=np.max(np.abs(proposed-z));z=proposed
 if delta<1e-6:break
def clip_polygon(points,axis,limit,positive):
 result=[]
 for a,b in zip(points,points[1:]+points[:1]):
  da=(a[axis]-limit)*(1 if positive else -1);db=(b[axis]-limit)*(1 if positive else -1);ina=da>=-1e-9;inb=db>=-1e-9
  if ina:result.append(a)
  if ina!=inb:result.append(a.lerp(b,da/(da-db)))
 return result
shells=[]
for bank in banks:
 verts=[];idx={};top=[];original_top=0
 for face in bank.data.polygons:
  if (bank.matrix_world.to_3x3().inverted().transposed()@face.normal).normalized().z<=.1:continue
  points=[bank.matrix_world@bank.data.vertices[k].co for k in face.vertices]
  if max(p.z for p in points)<.01:continue
  original_top+=1;pieces=[];current=points
  for axis,limit,positive in [(0,profile['x'][0],True),(0,profile['x'][1],False),(1,profile['y'][0],True),(1,profile['y'][1],False)]:
   if len(current)<3:break
   outside=clip_polygon(current,axis,limit,not positive)
   if len(outside)>=3:pieces.append(outside)
   current=clip_polygon(current,axis,limit,positive)
  for piece in pieces:
   row=[]
   for point in piece:
    key=tuple(round(float(v),5) for v in point)
    if key not in idx:idx[key]=len(verts);verts.append(tuple(point))
    if not row or row[-1]!=idx[key]:row.append(idx[key])
   if len(row)>1 and row[0]==row[-1]:row.pop()
   if len(set(row))>=3:
    area=sum((Vector(verts[row[i]])-Vector(verts[row[0]])).cross(Vector(verts[row[i+1]])-Vector(verts[row[0]])).length/2 for i in range(1,len(row)-1))
    if area>1e-7:top.append(tuple(row))
 assert top;closed_verts=[];allfaces=[];n=len(verts)
 for polygon in top:
  offset=len(closed_verts);points=[verts[i] for i in polygon];length=len(points);closed_verts.extend(points);closed_verts.extend((x,y,-1) for x,y,z0 in points);allfaces.append(tuple(offset+i for i in range(length)));allfaces.append(tuple(offset+length+i for i in reversed(range(length))))
  for i in range(length):j=(i+1)%length;allfaces.append((offset+i,offset+length+i,offset+length+j,offset+j))
 mesh=closed_mesh('Closed retained native bank prisms',closed_verts,allfaces)
 for mat in bank.data.materials:mesh.materials.append(mat)
 for vertex in mesh.vertices:vertex.co=bank.matrix_world.inverted()@vertex.co
 bank.data=mesh;shells.append(dict(node=bank['source_node'],original_top_faces=original_top,retained_top_pieces=len(top),top_vertices=n,method='Exact polygon clipping removes only rectangular joint footprint; original outside top coordinates retained. Each retained top piece has a closed underground prism atz=-1; coincident internal walls are buried. No Boolean solver.'))

verts=[(float(x),float(y),float(z[i,j])) for i,x in enumerate(xs) for j,y in enumerate(ys)];n=len(verts);ny=len(ys);verts += [(x,y,-1) for x,y,zz in verts];faces=[]
for i in range(len(xs)-1):
 for j in range(ny-1):
  a=i*ny+j;b=a+ny;faces += [(a,b,b+1,a+1),(n+a,n+a+1,n+b+1,n+b)]
boundary=list(range(ny))+[i*ny+ny-1 for i in range(1,len(xs))]+list(range((len(xs)-1)*ny+ny-2,(len(xs)-1)*ny-1,-1))+[i*ny for i in range(len(xs)-2,0,-1)]
for a,b in zip(boundary,boundary[1:]+boundary[:1]):faces.append((a,b,b+n,a+n))
mesh=closed_mesh('Continuous inferred bank joint',verts,faces);obj=bpy.data.objects.new(mesh.name,mesh);collection.objects.link(obj);obj['source_node']=f'tree{tree_id:02d}-local-soil-joint';obj['asset_group']=f'croisement01-tree{tree_id:02d}-local-soil';mat=bpy.data.materials.new('Unapproved inferred soil');mat.diffuse_color=(.32,.27,.17,1);mesh.materials.append(mat)
for face in mesh.polygons:face.use_smooth=True
assert all(geometry(o)==protected[o.get('source_node')] for o in collection.all_objects if o.type=='MESH' and o is not obj and o not in banks)
(worker/'modified').mkdir(parents=True);(worker/'inspection').mkdir();shutil.copy2(source/'modified/views.json',worker/'modified/views.json');(worker/'workspace.json').write_text(json.dumps(cfg,indent=2)+'\n');bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'));assert sha(source/'model.blend')==before
(dest/'construction.json').write_text(json.dumps(dict(status='Private joint geometry hypothesis, not approved',source_sha256=before,model_sha256=sha(worker/'model.blend'),wood_and_other_context_unchanged=True,terrain_changed=profile['nodes']+[f'tree{tree_id:02d}-local-soil-joint'],shell_repair=shells,bounds=[profile['x'],profile['y']],inferred='Smooth soil to exact original perimeter heights; under-root height constrained to lowest actual wood surface minus0.3, rather than a guessed flat seat. Hidden terrain inferred, gameplay untouched.',iterations=it+1,convergence=float(delta),max_raise=float(np.max(z-ground)),max_lower=float(np.max(ground-z)),native_visibility_ray_constraints=constraints,visibility_margin=.5),indent=2)+'\n');print(worker)
