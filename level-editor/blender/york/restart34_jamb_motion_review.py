"""Verify saved native UV sampling and render the private gate/jamb motion together."""
import hashlib,json,math,shutil,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];W=ROOT/'level-editor/work/york-refinement/restart2';OUT=W/'jamb-clearance-candidate-v1/motion-review'
assert not OUT.exists()
assert shutil.disk_usage(ROOT).free>10*1024**3
assert int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')))*1024>6*1024**3
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy,numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from mathutils.geometry import barycentric_transform
from PIL import Image,ImageDraw
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from render_views import render_views
mask=np.array(Image.open(W/'jamb-source-probe-v1/visible-jamb-domain.png').convert('L'))>0
s=math.sin(math.radians(35));c=math.cos(math.radians(35));back=Vector((0,-c,s))
coords=[(int(x)+2250,int(y)+780) for y,x in np.argwhere(mask)]
def source_samples(path):
 bpy.ops.wm.open_mainfile(filepath=str(path));bpy.context.view_layer.update();o=bpy.data.objects['building-778-portcullis-jamb-return'];m=o.data;m.calc_loop_triangles();vs=[o.matrix_world@v.co for v in m.vertices];tris=list(m.loop_triangles);tree=BVHTree.FromPolygons(vs,[tuple(t.vertices) for t in tris],all_triangles=True);result=[]
 for x,y in coords:
  loc,n,ti,d=tree.ray_cast(Vector((x+.5,-(y+.5)/s,0))+back*10000,-back)
  assert loc is not None
  t=tris[ti];v=[vs[i] for i in t.vertices];uv=[Vector((*m.uv_layers.active.data[i].uv,0)) for i in t.loops];p=barycentric_transform(loc,*v,*uv)
  result.append({'triangle':ti,'material':t.material_index,'uv':[p.x,p.y]})
 return result
before=source_samples(W/'jamb-textures-v1/model.blend')
after=source_samples(W/'jamb-clearance-candidate-v1/model.blend')
assert all(a['triangle']==b['triangle'] and a['material']==b['material'] for a,b in zip(before,after))
err=max(abs(a['uv'][k]-b['uv'][k]) for a,b in zip(before,after) for k in (0,1));assert err<2e-5
scene=bpy.context.scene;scene.render.threads_mode='FIXED';scene.render.threads=2
jamb=bpy.data.objects['building-778-portcullis-jamb-return']
# Use the exact approved gate mesh/material, replacing only the temporary review context.
source=W/'gate-textures-v1/covered/model.blend'
assert hashlib.sha256(source.read_bytes()).hexdigest()=='6ed0064f493f79cb38f249fdbc41177eac3cee5659fb86c73f59ae746ad19b70'
with bpy.data.libraries.load(str(source),link=False) as (available,loaded):loaded.objects=['scenery-york-castle-portcullis']
gate=loaded.objects[0];scene.collection.objects.link(gate)
for o in scene.objects:
 if o.type=='MESH':o.hide_render=o not in (gate,jamb)
bpy.context.view_layer.update()
audit=json.loads((W/'gate-saved-contact-audit-v1/report.json').read_text());expected=audit['geometry_world']['gate_vertices'];actual=[list(gate.matrix_world@v.co) for v in gate.data.vertices];assert actual==expected,'Appended gate world transform changed'
base=gate.matrix_world.copy();motion=json.loads((W/'gate-motion-proposal-v2/motion.json').read_text())
OUT.mkdir();scene.render.resolution_x=220;scene.render.resolution_y=250
for row in motion['rows']:
 gate.matrix_world=base.copy();gate.matrix_world.translation.z+=row['nominal_lift_world_z'];bpy.context.view_layer.update()
 render_views(scene.name,{'native':'Native crop'},OUT/f'frame-{row["frame"]:02}',modes=('textured',),width=220)
# Native frame sequence, keeping frame 36 inference explicit.
sheet=Image.new('RGBA',(220*9,274*5),(35,39,45,255));draw=ImageDraw.Draw(sheet)
for row in motion['rows']:
 i=row['frame'];im=Image.open(OUT/f'frame-{i:02}/native-textured.png').convert('RGBA');x=(i%9)*220;y=(i//9)*274;sheet.alpha_composite(im,(x,y+24));draw.text((x+4,y+4),f'{i}: lift {row["nominal_lift_source_pixels"]}'+(' INFERRED' if i==36 else ''),fill='white')
sheet.save(OUT/'native-45-poses.png')
# Orthographic contact views of initial and final gate depth, including underside.
points=[jamb.matrix_world@v.co for v in jamb.data.vertices];center=sum(points,Vector())/len(points);cameras={}
for i,(yaw,elev) in enumerate([(0,35),(90,20),(180,35),(270,20),(0,-30),(90,-30),(180,-30),(270,-30)]):
 a,b=math.radians(yaw),math.radians(elev);direction=Vector((math.sin(a)*math.cos(b),-math.cos(a)*math.cos(b),math.sin(b)));data=bpy.data.cameras.new('Gate contact '+str(i));camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera);data.type='ORTHO';data.ortho_scale=210;data.clip_end=20000;camera.location=center+direction*10000;camera.rotation_euler=(-direction).to_track_quat('-Z','Y').to_euler();cameras[f'view-{i}']=camera.name
scene.render.resolution_x=320;scene.render.resolution_y=384
for frame in (0,22,34,36,38,44):
 row=motion['rows'][frame];gate.matrix_world=base.copy();gate.matrix_world.translation.z+=row['nominal_lift_world_z'];bpy.context.view_layer.update();dest=OUT/f'contact-{frame:02}';render_views(scene.name,cameras,dest,modes=('solid','textured'),width=240)
 for mode in ('solid','textured'):
  views=[Image.open(dest/f'view-{i}-{mode}.png').convert('RGBA') for i in range(8)];sheet=Image.new('RGBA',(views[0].width*4,views[0].height*2))
  for i,view in enumerate(views):sheet.paste(view,((i%4)*view.width,(i//4)*view.height))
  sheet.save(dest/f'{mode}-eight.png')
report={'status':'SAVED_SOURCE_UV_AND_MOTION_REVIEW_EVIDENCE','source_pixels':660,'same_actual_loop_triangle_and_material':660,'maximum_saved_uv_drift':err,'gate_review_geometry_exact':True,'frame36':'Inferred 57-pixel lift within57–65 source uncertainty, not an exact measured hidden pose','native_sequence_frames':45,'contact_eight_views_at_frames':[0,22,34,36,38,44],'native_shaded_raster_changes':'Shaded context comparison differs; exact front vertices, texture bytes and UV loops retained, with independently checked saved source UV sampling. Do not claim byte-identical shaded renders.'}
(OUT/'report.json').write_text(json.dumps(report,indent=2)+'\n');assert sum(p.stat().st_size for p in OUT.parent.rglob('*') if p.is_file())<32*1024**2;print(json.dumps(report))
