"""Diagnostic source-aperture cut only; ray-aligned depth is not a final physical wall."""
import json,math,sys,hashlib
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement';OUT=WORK/'restart2/winch-room-probe-v1'
if OUT.exists():raise FileExistsError(OUT)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
from mathutils import Vector
from PIL import Image
import numpy as np
from collections import deque
source=WORK/'restart2/winch-geometry-v2/transition-44/model.blend';bpy.ops.wm.open_mainfile(filepath=str(source));scene=bpy.context.scene;wall=next(o for o in scene.objects if o.get('source_node')=='building-776')
a=np.array(Image.open(WORK/'baseline/masks/000615.png').convert('L'))>0;seed=(150,40);assert not a[seed];q=deque([seed]);seen={seed}
while q:
 y,x=q.popleft()
 for dy,dx in [(0,1),(0,-1),(1,0),(-1,0)]:
  yy,xx=y+dy,x+dx
  if 0<=yy<a.shape[0] and 0<=xx<a.shape[1] and not a[yy,xx] and (yy,xx) not in seen:seen.add((yy,xx));q.append((yy,xx))
assert len(seen)==5699
rows={y:sorted(x for yy,x in seen if yy==y) for y in sorted({p[0] for p in seen})};ys=list(rows);ys=ys[::3]+[ys[-1]]
profile=[(2353+rows[y][0],783+y) for y in ys]+[(2353+rows[y][-1]+1,784+y) for y in reversed(ys)]
s,c=math.sin(math.radians(35)),math.cos(math.radians(35));back=Vector((0,-c,s));points=[Vector((x,-y/s,0)) for x,y in profile];n=len(points);v=points+[p+back*1200 for p in points];f=[tuple(reversed(range(n))),tuple(range(n,n*2))]+[(i,(i+1)%n,(i+1)%n+n,i+n) for i in range(n)]
mesh=bpy.data.meshes.new('Native aperture diagnostic cutter');mesh.from_pydata(v,[],f);mesh.update();cut=bpy.data.objects.new(mesh.name,mesh);scene.collection.objects.link(cut)
protected={o.name:hashlib.sha256(json.dumps([list(o.matrix_world@v.co) for v in o.data.vertices]).encode()).hexdigest() for o in scene.objects if o.type=='MESH' and o not in [wall,cut]}
bpy.context.view_layer.objects.active=wall;mod=wall.modifiers.new('Diagnostic native room aperture','BOOLEAN');mod.operation='DIFFERENCE';mod.solver='EXACT';mod.object=cut;bpy.ops.object.modifier_apply(modifier=mod.name);bpy.data.objects.remove(cut,do_unlink=True);assert len(wall.data.polygons)>0
for name,digest in protected.items():
 o=bpy.data.objects[name];assert hashlib.sha256(json.dumps([list(o.matrix_world@v.co) for v in o.data.vertices]).encode()).hexdigest()==digest
OUT.mkdir(parents=True);(OUT/'transition-44').mkdir();bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'transition-44/model.blend'),compress=True)
(OUT/'proposal.json').write_text(json.dumps({'status':'DIAGNOSTIC ONLY, ray-aligned cut is not final physical architecture','changed_node':'building-776','source_mask':615,'source_hole_pixels':5699,'source_profile':profile,'protected_meshes':protected,'purpose':'Distinguish solid proxy occlusion from wrong winch placement; next derive physical jamb/soffit depth from visible source and existing footprint.'},indent=2)+'\n');print('ROOM PROBE SAVED')
