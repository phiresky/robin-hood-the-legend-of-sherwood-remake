"""Private patch005 ownership split of the protected room-facing wall only."""
import json,math,sys,hashlib,shutil
from pathlib import Path
from collections import deque
ROOT=Path(__file__).resolve().parents[3];W=ROOT/'level-editor/work/york-refinement/restart2';OUT=W/'restart61-room-split-v6';assert not OUT.exists();assert shutil.disk_usage(ROOT).free>10*1024**3;sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
import shutil
assert shutil.disk_usage(ROOT).free>10*1024**3
assert int(next(x.split()[1]for x in Path('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')))*1024>6*1024**3
acquire(slots=2)
import bpy,bmesh,numpy as np
from mathutils import Vector
from PIL import Image
base=W/'restart42-channel-assembly-v1/model.blend';bpy.ops.wm.open_mainfile(filepath=str(base));scene=bpy.context.scene;wall=next(o for o in scene.objects if o.type=='MESH' and o.get('source_node')=='building-776');bpy.context.view_layer.update();walls=[o for o in scene.objects if o.type=='MESH' and o.get('source_node')in {'building-764','building-765','building-776','building-779'}];assert len(walls)==4;protected={o.name:hashlib.sha256(json.dumps([[list(v.co)for v in o.data.vertices],[list(r)for r in o.matrix_world]]).encode()).hexdigest() for o in scene.objects if o.type=='MESH' and o not in walls}
a=np.array(Image.open(W.parent/'baseline/masks/000615.png').convert('L'))>0;q=deque([(150,40)]);seen=set(q)
while q:
 y,x=q.popleft()
 for yy,xx in [(y+1,x),(y-1,x),(y,x+1),(y,x-1)]:
  if 0<=yy<a.shape[0] and 0<=xx<a.shape[1] and not a[yy,xx] and (yy,xx)not in seen:seen.add((yy,xx));q.append((yy,xx))
assert len(seen)==5699
level=json.loads((W.parent/'baseline/york.rhp.json').read_text());foot=level['sight_obstacles'][776]['points'];s,c=math.sin(math.radians(35)),math.cos(math.radians(35))
def front_y(x):
 hits=[]
 for a,b in zip(foot,foot[1:]+foot[:1]):
  if min(a['x'],b['x'])<=x<=max(a['x'],b['x']) and abs(a['x']-b['x'])>1e-6:hits.append(a['y']+(x-a['x'])/(b['x']-a['x'])*(b['y']-a['y']))
 assert hits;return max(hits)
cols={x:sorted(y for y,xx in seen if xx==x)for x in sorted({p[1]for p in seen})};xs=sorted(cols);v=[];f=[]
for x in xs:
 px=2353+x+.5;fy=front_y(px);zt=(fy-783-cols[x][0])/c;zb=(fy-784-cols[x][-1])/c
 v.extend([(px,-(fy+.1)/s,zb),(px,-(fy+.1)/s,zt),(px,-(fy-120)/s,zt),(px,-(fy-120)/s,90.001/c)])
for j in range(len(xs)-1):
 n=j*4;m=n+4;f.extend([(n,m,m+1,n+1),(n+1,m+1,m+2,n+2),(n+2,m+2,m+3,n+3),(n+3,m+3,m,n)])
f.extend([(3,2,1,0),tuple(range(len(v)-4,len(v)))]);mesh=bpy.data.meshes.new('Mask615 aperture prism');mesh.from_pydata(v,[],f);mesh.update();bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.triangulate(bm,faces=list(bm.faces));bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(mesh);bm.free();cut=bpy.data.objects.new(mesh.name,mesh);scene.collection.objects.link(cut)
# Surface partition works for open presentation meshes. Each clip vertex carries
# all source UV layers; partitioning never asks a solid Boolean to infer volume.
def split(poly,normal,d):
 inside=[];outside=[]
 for aa,bb in zip(poly,poly[1:]+poly[:1]):
  da=aa[0].dot(normal)-d;db=bb[0].dot(normal)-d;ain=da<=1e-7;bin=db<=1e-7
  (inside if ain else outside).append(aa)
  if ain!=bin:
   t=da/(da-db);vv=(aa[0].lerp(bb[0],t),[u.lerp(v,t) for u,v in zip(aa[1],bb[1])]);inside.append(vv);outside.append(vv)
 return inside,outside
slabs=[]
# Ownership boundaries follow the exact native aperture pixels. The existing
# surfaces remain physical surfaces; this partitions their source-facing domain.
for xx,yy in cols.items():
 left=2353+xx;right=left+1;top=783+min(yy);bottom=784+max(yy)
 slabs.append([(Vector((1,0,0)),right),(Vector((-1,0,0)),-left),(Vector((0,-s,-c)),bottom),(Vector((0,s,c)),-top)])
def make_mesh(name,polys,materials,uvnames):
 verts=[];faces=[]
 for poly,material,smooth in polys:
  faces.append(tuple(range(len(verts),len(verts)+len(poly))));verts.extend(tuple(a[0])for a in poly)
 m=bpy.data.meshes.new(name);m.from_pydata(verts,[],faces);m.update()
 for material in materials:m.materials.append(material)
 layers=[m.uv_layers.new(name=n)for n in uvnames]
 for face,(poly,material,smooth)in zip(m.polygons,polys):
  face.material_index=material;face.use_smooth=smooth
  for li,vv in zip(face.loop_indices,poly):
   for k,layer in enumerate(layers):layer.data[li].uv=vv[1][k]
 return m
covers=[];counts=[]
for wall in walls:
 wall.data.calc_loop_triangles();remaining=[];owned=[];uvnames=[u.name for u in wall.data.uv_layers];materials=list(wall.data.materials)
 for tri in wall.data.loop_triangles:
  poly=[(wall.matrix_world@wall.data.vertices[wall.data.loops[li].vertex_index].co,[u.data[li].uv.copy()for u in wall.data.uv_layers])for li in tri.loops];face=wall.data.polygons[tri.polygon_index];remaining.append((poly,face.material_index,face.use_smooth))
 for planes in slabs:
  next_remaining=[]
  for poly,mat,smooth in remaining:
   current=poly
   for normal,d in planes:
    current,outside=split(current,normal,d)
    if len(outside)>=3:next_remaining.append((outside,mat,smooth))
    if len(current)<3:break
   if len(current)>=3:owned.append((current,mat,smooth))
  remaining=next_remaining
 assert owned and remaining
 cover=wall.copy();cover.name='Room cover / switched '+wall['source_node'];cover.data=make_mesh(cover.name,owned,materials,uvnames);scene.collection.objects.link(cover);cover.parent=None;cover.matrix_world.identity();cover['native_patch']='patch-005';cover['source_node']='mission-york-room-cover';cover['state_recipe']='initial_only';covers.append(cover)
 wall.data=make_mesh(wall.name+' partition',remaining,materials,uvnames);wall.parent=None;wall.matrix_world.identity();counts.append({'wall':wall.name,'remainder':len(remaining),'cover':len(owned)})
bpy.data.objects.remove(cut,do_unlink=True);bpy.context.view_layer.update()
for name,digest in protected.items():
 ob=scene.objects[name];assert hashlib.sha256(json.dumps([[list(v.co)for v in ob.data.vertices],[list(r)for r in ob.matrix_world]]).encode()).hexdigest()==digest
OUT.mkdir();bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'model.blend'),compress=True);(OUT/'report.json').write_text(json.dumps({'status':'PRIVATE_SPLIT_PENDING_GEOMETRY_SOURCE_CONTACT_REVIEW','model_sha256':hashlib.sha256((OUT/'model.blend').read_bytes()).hexdigest(),'mask_hole_pixels':5699,'changed_static_source_nodes':['building-764','building-765','building-776','building-779'],'unrelated_meshes_exact':len(protected),'remainder_faces':sum(len(o.data.polygons)for o in walls),'cover_faces':sum(len(o.data.polygons)for o in covers),'inference':'Existing surfaces partitioned by exact mask615 aperture rays. Hidden room depth belongs to separately inferred interior solids; this partition is source ownership, not a measured cavity depth.','initial_union':'World-space triangle partition with interpolated source UVs; quantitative ray equality audit pending.','partition_counts':counts},indent=2)+'\n')
