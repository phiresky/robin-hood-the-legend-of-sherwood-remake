"""Private physical arch opening proposal through separately scoped room receivers."""
import json,math,sys,hashlib
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement';OUT=WORK/'restart2/winch-room-physical-v7'
if OUT.exists():raise FileExistsError(OUT)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
from mathutils import Vector
from PIL import Image
import numpy as np
from collections import deque
initial_source=WORK/'restart2/winch-geometry-v6/transition-00/model.blend';bpy.ops.wm.open_mainfile(filepath=str(initial_source));bpy.context.view_layer.update();initial_pose={o.name:([list(r) for r in o.matrix_world],o.hide_render) for o in bpy.context.scene.objects if o.get('native_patch')=='patch-004'}
source=WORK/'restart2/winch-geometry-v6/transition-44/model.blend';bpy.ops.wm.open_mainfile(filepath=str(source));scene=bpy.context.scene;wall=next(o for o in scene.objects if o.get('source_node')=='building-776')
a=np.array(Image.open(WORK/'baseline/masks/000615.png').convert('L'))>0;seed=(150,40);assert not a[seed];q=deque([seed]);seen={seed}
while q:
 y,x=q.popleft()
 for dy,dx in [(0,1),(0,-1),(1,0),(-1,0)]:
  yy,xx=y+dy,x+dx
  if 0<=yy<a.shape[0] and 0<=xx<a.shape[1] and not a[yy,xx] and (yy,xx) not in seen:seen.add((yy,xx));q.append((yy,xx))
assert len(seen)==5699
rows={y:sorted(x for yy,x in seen if yy==y) for y in sorted({p[0] for p in seen})};ys=list(rows);ys=ys[::3]+[ys[-1]]
profile=[(2353+rows[y][0],783+y) for y in ys]+[(2353+rows[y][-1]+1,784+y) for y in reversed(ys)]
s,c=math.sin(math.radians(35)),math.cos(math.radians(35))
level=json.loads((WORK/'baseline/york.rhp.json').read_text())
nodes={f'building-{i}' for i in (764,765,776,779)}
changed=[o for o in scene.objects if o.type=='MESH' and o.get('source_node') in nodes]
assert len(changed)==4
protected={o.name:hashlib.sha256(json.dumps([list(o.matrix_world@v.co) for v in o.data.vertices]).encode()).hexdigest() for o in scene.objects if o.type=='MESH' and o not in changed}
foot=level['sight_obstacles'][776]['points']
def front_y(x):
 hits=[]
 for a,b in zip(foot,foot[1:]+foot[:1]):
  if min(a['x'],b['x'])<=x<=max(a['x'],b['x']) and abs(a['x']-b['x'])>1e-6:
   t=(x-a['x'])/(b['x']-a['x']);hits.append(a['y']+t*(b['y']-a['y']))
 assert hits,x
 return max(hits)
# Boundary height derives from the visible front wall's footprint, not camera-ray
# depth. Horizontal extrusion provides vertical jambs and a horizontal arch tunnel.
# Floor clearance extends down to the native room floor; sill ownership is pending.
cols={x:sorted(y for y,xx in seen if xx==x) for x in sorted({p[1] for p in seen})}
xs=list(cols);xs=sorted(set(xs[::2]+[xs[-1]]))
arch_top=[(2353+x,783+cols[x][0]) for x in xs]
v=[];f=[]
for x,y in arch_top:
 fy=front_y(x);z=(fy-y)/c
 v.extend([Vector((x,-(fy+12)/s,90.00101/c)),Vector((x,-(fy+12)/s,z)),Vector((x,-(fy-120)/s,z)),Vector((x,-(fy-120)/s,90.00101/c))])
for j in range(len(arch_top)-1):
 n=j*4;m=n+4
 f.extend([(n,m,m+1,n+1),(n+1,m+1,m+2,n+2),(n+2,m+2,m+3,n+3),(n+3,m+3,m,n)])
f.extend([(3,2,1,0),tuple(range(len(v)-4,len(v)))])
mesh=bpy.data.meshes.new('Physical room opening cutter');mesh.from_pydata(v,[],f);mesh.update();cut=bpy.data.objects.new(mesh.name,mesh);scene.collection.objects.link(cut)
import bmesh
bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.triangulate(bm,faces=list(bm.faces));bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(mesh);bm.free()
for wall in changed:
 record=level['sight_obstacles'][int(wall['source_node'].split('-')[1])];pts=record['points'];count=len(pts)
 vv=[Vector((p['x'],-p['y']/s,p[key]/c)) for key in ['z_bottom','z_top'] for p in pts]
 ff=[tuple(reversed(range(count))),tuple(range(count,count*2))]+[(i,(i+1)%count,(i+1)%count+count,i+count) for i in range(count)]
 material=wall.data.materials[0];clean=bpy.data.meshes.new(wall.name+' closed source volume');clean.from_pydata([wall.matrix_world.inverted()@p for p in vv],[],ff);clean.update();wall.data=clean;wall.data.materials.append(material)
 bm=bmesh.new();bm.from_mesh(clean);bmesh.ops.triangulate(bm,faces=list(bm.faces));bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));assert all(e.is_manifold for e in bm.edges);bm.to_mesh(clean);bm.free()
 bpy.context.view_layer.objects.active=wall;mod=wall.modifiers.new('Scoped room opening','BOOLEAN');mod.operation='DIFFERENCE';mod.solver='EXACT';mod.object=cut;bpy.ops.object.modifier_apply(modifier=mod.name);assert len(wall.data.polygons)>0
bpy.data.objects.remove(cut,do_unlink=True)
for name,digest in protected.items():
 o=bpy.data.objects[name];assert hashlib.sha256(json.dumps([list(o.matrix_world@v.co) for v in o.data.vertices]).encode()).hexdigest()==digest
OUT.mkdir(parents=True);(OUT/'transition-44').mkdir();bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'transition-44/model.blend'),compress=True)
from mathutils import Matrix
for name,(matrix,hidden) in initial_pose.items():
 o=scene.objects[name];o.matrix_world=Matrix(matrix);o.hide_render=hidden
bpy.context.view_layer.update()
for name,(matrix,hidden) in initial_pose.items():
 o=scene.objects[name];assert max(abs(o.matrix_world[r][c]-matrix[r][c]) for r in range(4) for c in range(4))<1e-6;assert o.hide_render==hidden
(OUT/'transition-00').mkdir();bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'transition-00/model.blend'),compress=True)
(OUT/'proposal.json').write_text(json.dumps({'status':'HOLD physical cavity proposal requires source/contact review','changed_nodes':sorted(nodes),'source_mask':615,'source_hole_pixels':5699,'source_profile':profile,'protected_meshes':protected,'protection_scope':'Final-state carving changes only4room nodes; initial endpoint then restores exact authored initial mechanism transforms.','initial_pose_source_sha256':hashlib.sha256(initial_source.read_bytes()).hexdigest(),'final_pose_source_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'states':{state:hashlib.sha256((OUT/state/'model.blend').read_bytes()).hexdigest() for state in ('transition-00','transition-44')},'purpose':'Source arch traced onto physical front wall776 footprint, horizontally extruded through four overlapping room proxies. Hidden depth120game and front padding12game are explicit inference; native floor90 retained; source foreground sill remains separate ownership to reconcile.'},indent=2)+'\n');print('ROOM PROBE SAVED')
