"""Build a private solid removable room cover with observed front and inferred back."""
import json,hashlib,math,sys,shutil
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];W=ROOT/'level-editor/work/active/york/restart2';OUT=W/'restart61-room-geometry-v2';assert not OUT.exists()
assert shutil.disk_usage(ROOT).free>10*1024**3
assert int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')))*1024>6*1024**3
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire(slots=2)
import bpy,bmesh
import numpy as np
from mathutils import Vector
from PIL import Image
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from render_views import render_views
source=json.loads((W/'restart61-room-source-v1/source.json').read_text());path=ROOT/source['frame_path'];rgba=np.array(Image.open(path));assert hashlib.sha256(path.read_bytes()).hexdigest()==source['frame_sha256']
bpy.ops.wm.read_factory_settings(use_empty=True);scene=bpy.context.scene;scene.render.engine='CYCLES';scene.cycles.samples=12;scene.cycles.use_denoising=False;scene.render.threads_mode='FIXED';scene.render.threads=2;scene.render.film_transparent=True;scene.view_settings.view_transform='Standard';scene.render.resolution_x=256;scene.render.resolution_y=320
s,c=math.sin(math.radians(35)),math.cos(math.radians(35));floor=90.001/c;verts=[];faces=[];front_count=4;trace=[]
# Each image column constrains top and bottom only; the common floor solves
# horizontal depth. Six game units of rear thickness are explicitly inferred.
for i,top,bottom in [(0,12,76),(8,1,86),(24,3,106),(59,5,113),(79,20,125)]:
 gamey=876+bottom+90.001
 trace.append({'x':2356+i,'top_y':876+top,'bottom_y':876+bottom,'floor_game_z':90.001,'inferred_game_y':gamey})
 verts.extend([(2356+i,-gamey/s,floor),(2356+i,-gamey/s,(gamey-876-top)/c),(2356+i,-(gamey-6)/s,(gamey-876-top)/c),(2356+i,-(gamey-6)/s,floor)])
for i in range(4):
 n=i*4;m=n+4;faces.append((n,m,m+1,n+1))
for i in range(4):
 n=i*4;m=n+4;faces.extend([(n+1,m+1,m+2,n+2),(n+2,m+2,m+3,n+3),(n+3,m+3,m,n)])
faces.extend([(3,2,1,0),(16,17,18,19)])
mesh=bpy.data.meshes.new('Room cover solid folded masonry');mesh.from_pydata(verts,[],faces);mesh.update();o=bpy.data.objects.new(mesh.name,mesh);scene.collection.objects.link(o);o['source_node']='mission-york-room-cover';o['native_patch']='patch-005';o['state_recipe']='initial_only';o['reveal_hide_when_applied']='patch-005';o['source_image_sha256']=source['frame_sha256']
mat=bpy.data.materials.new('Exact native initial cover front');mat.use_nodes=True;nodes=mat.node_tree.nodes;nodes.clear();tex=nodes.new('ShaderNodeTexImage');tex.image=bpy.data.images.load(str(path));tex.image.pack();tex.interpolation='Closest';em=nodes.new('ShaderNodeEmission');output=nodes.new('ShaderNodeOutputMaterial');mat.node_tree.links.new(tex.outputs['Color'],em.inputs['Color']);mat.node_tree.links.new(em.outputs[0],output.inputs[0]);mesh.materials.append(mat)
unknown=bpy.data.materials.new('Inferred hidden stone pending texture');unknown.diffuse_color=(.28,.28,.28,1);unknown.use_nodes=True;unknown.node_tree.nodes.get('Principled BSDF').inputs['Base Color'].default_value=(.28,.28,.28,1);mesh.materials.append(unknown)
uv=mesh.uv_layers.new(name='Native patch projection')
for poly in mesh.polygons:
 poly.material_index=0 if poly.index<front_count else 1
 for li in poly.loop_indices:
  v=mesh.vertices[mesh.loops[li].vertex_index].co;screeny=-v.y*s-v.z*c;uv.data[li].uv=((v.x-2356)/79,1-(screeny-876)/125)
bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));assert all(e.is_manifold for e in bm.edges);volume=bm.calc_volume(signed=False);assert volume>0;bm.to_mesh(mesh);bm.free()
world=bpy.data.worlds.new('Neutral review world');scene.world=world;world.use_nodes=True;world.node_tree.nodes['Background'].inputs[0].default_value=(.7,.7,.7,1)
points=[Vector(v) for v in verts];cameras={}
for i,yaw in enumerate(range(0,360,45)):
 a=math.radians(yaw);back=Vector((math.sin(a)*c,-math.cos(a)*c,s));q=(-back).to_track_quat('-Z','Y');right=q@Vector((1,0,0));up=q@Vector((0,1,0));rx=[p.dot(right) for p in points];uy=[p.dot(up) for p in points];dz=[p.dot(back) for p in points];center=right*((min(rx)+max(rx))/2)+up*((min(uy)+max(uy))/2)+back*((min(dz)+max(dz))/2);d=bpy.data.cameras.new('Native source camera' if i==0 else f'Oblique {i}');cam=bpy.data.objects.new(d.name,d);scene.collection.objects.link(cam);d.type='ORTHO';d.clip_end=20000;d.ortho_scale=max(max(uy)-min(uy),(max(rx)-min(rx))*320/256)*1.15;cam.location=center+back*10000;cam.rotation_euler=q.to_euler();cameras[f'view-{i}']=cam.name
OUT.mkdir();bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'model.blend'),compress=True)
render_views(scene.name,cameras,OUT/'views',modes=('textured','solid'),width=256)
for mode in ['textured','solid']:
 sheet=Image.new('RGBA',(1024,640))
 for i in range(8):sheet.paste(Image.open(OUT/f'views/view-{i}-{mode}.png'),((i%4)*256,(i//4)*320))
 sheet.save(OUT/f'{mode}-eight.png')
(OUT/'report.json').write_text(json.dumps({'status':'PRIVATE_CANDIDATE_PENDING_CONTACT_AND_SELF_REVIEW','model_sha256':hashlib.sha256((OUT/'model.blend').read_bytes()).hexdigest(),'source_sha256':source['frame_sha256'],'native_patch':'patch-005','physical_applied':'absent','trace':trace,'inferred_floor_game_z':90.001,'inferred_wall_thickness_game_y':6,'manifold':True,'volume':volume,'source_pixel_policy':'Native image RGB mapped to observed front. No synthesis. Rear, ends, upper and lower wall thickness are neutral unknown.','limitations':['Piecewise planar silhouette and common floor depth are hypotheses, not measured wall footprint. Left lower corner extends beneath source-alpha occlusion.','Room interior receivers are outside this candidate.','Protected gatehouse and moving winch contact still pending.']},indent=2)+'\n')
print('ROOM COVER CANDIDATE SAVED',OUT)
