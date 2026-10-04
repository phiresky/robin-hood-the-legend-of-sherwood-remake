"""Prove native painted sign shadows on synchronized horizontal receiver planes."""
import json,sys,math
from pathlib import Path
import bpy
import numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from tree_geometry import SIN,COS,RAY
from evidence_io import sha,write_json
from render_slots import acquire,release

def constant_action(obj):
 for layer in obj.animation_data.action.layers:
  for strip in layer.strips:
   for bag in strip.channelbags:
    for curve in bag.fcurves:
     for key in curve.keyframe_points:key.interpolation='CONSTANT'

def main():
 base=OUT/'state-sign-candidate';source=base/'native-fill-v1/model.blend';source_hash=sha(source);proposal=base/'painted-shadow-v2/proposal.json';record=json.loads(proposal.read_text());dst=base/'shadow-proof-v2';dst.mkdir(exist_ok=False);bpy.ops.wm.open_mainfile(filepath=str(source));scene=bpy.context.scene;shadows=[];z=.03
 for row in record['records']:
  i=row['frame'];path=Path(row['mask']);assert sha(path)==row['mask_sha256'];mesh=bpy.data.meshes.new(f'Native shadow receiver {i:02}');mesh.from_pydata([(-32,(55-COS*z)/SIN,z),(32,(55-COS*z)/SIN,z),(32,(-17-COS*z)/SIN,z),(-32,(-17-COS*z)/SIN,z)],[],[(3,2,1,0)]);uv=mesh.uv_layers.new(name='Native shadow UV');coords=[(0,1),(1,1),(1,0),(0,0)]
  for loop in mesh.loops:uv.data[loop.index].uv=coords[loop.vertex_index]
  obj=bpy.data.objects.new(mesh.name,mesh);scene.collection.objects.link(obj);obj['asset_group']='croisement02-mission-rotating-sign';obj['source_node']='mission-panneau-shadow';obj['native_frame']=i;obj['mission_profile']='Panneau';obj['mission_visibility']='S03_FoB_MP';obj['appearance_provenance']='Proposed native painted-ground shadow subset; source RGB exact'
  mat=bpy.data.materials.new(f'Native black ground stroke {i:02}');mat.use_nodes=True;n=mat.node_tree.nodes;n.clear();tex=n.new('ShaderNodeTexImage');tex.image=bpy.data.images.load(str(path),check_existing=False);tex.image.pack();tex.interpolation='Closest';emit=n.new('ShaderNodeEmission');emit.inputs[0].default_value=(0,0,0,1);transparent=n.new('ShaderNodeBsdfTransparent');mix=n.new('ShaderNodeMixShader');out=n.new('ShaderNodeOutputMaterial');links=mat.node_tree.links;links.new(tex.outputs['Alpha'],mix.inputs[0]);links.new(transparent.outputs[0],mix.inputs[1]);links.new(emit.outputs[0],mix.inputs[2]);links.new(mix.outputs[0],out.inputs[0]);mat['foliage_physical_opacity']=True;mat['foliage_alpha_cutoff']=.5;mat['opacity_semantics']='physical-coverage';mesh.materials.append(mat)
  for frame,value in sorted({1:1 if i==0 else 0,1+2*i:1,3+2*i:0,65:1 if i==0 else 0}.items()):obj.scale=(value,value,value);obj.keyframe_insert(data_path='scale',frame=frame)
  constant_action(obj);shadows.append(obj)
 scene.frame_set(1);scene.frame_end=64;scene.cycles.samples=8;scene.cycles.use_denoising=False;scene.cycles.transparent_max_bounces=128;bpy.ops.wm.save_as_mainfile(filepath=str(dst/'model.blend'));digest=sha(dst/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(dst/'model.blend'));scene=bpy.context.scene;shadows=[scene.objects[f'Native shadow receiver {i:02}']for i in range(32)];data=bpy.data.cameras.new('Native sign source camera');data.type='ORTHO';data.sensor_fit='HORIZONTAL';data.ortho_scale=64;camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera);scene.camera=camera;target=Vector((0,19/SIN,0));camera.location=target+RAY*500;camera.rotation_euler=(target-camera.location).to_track_quat('-Z','Y').to_euler();scene.render.resolution_x=256;scene.render.resolution_y=288;scene.render.resolution_percentage=100;views=dst/'poses';views.mkdir();sheet=Image.new('RGB',(8*192,4*240),(80,80,80));stats=[];gif=[]
 for i,row in enumerate(record['records']):
  scene.frame_set(1+2*i);visible=[o for o in shadows if o.scale.x>.5];assert [o['native_frame']for o in visible]==[i];scene.render.filepath=str(views/f'pose-{i:02}.png');bpy.ops.render.render(write_still=True);render=Image.open(scene.render.filepath).convert('RGBA');rgba=np.asarray(render)[2::4,2::4];expected=np.asarray(Image.open(row['mask']).convert('RGBA'))[:,:,3]>127;black=(rgba[:,:,:3].max(2)<12)&(rgba[:,:,3]>127);stats.append(dict(frame=i,expected_shadow_pixels=int(expected.sum()),actual_black_shadow_pixels=int((black&expected).sum()),missing=int((expected&~black).sum())));tile=Image.new('RGB',render.size,(80,80,80));tile.paste(render,mask=render.getchannel('A'));gif.append(tile);sheet.paste(tile.resize((192,216),Image.Resampling.NEAREST),((i%8)*192,(i//8)*240));ImageDraw.Draw(sheet).text(((i%8)*192+3,(i//8)*240+220),str(i),fill='white')
 sheet.save(dst/'actual-32-poses.png');gif[0].save(dst/'native-motion.gif',save_all=True,append_images=gif[1:],duration=80,loop=0);assert sha(source)==source_hash;write_json(dst/'evidence.json',dict(status='Private synchronized painted-ground proof; independent visual/native-order review pending',source_model_sha256=source_hash,model_sha256=digest,proposal_sha256=sha(proposal),native_pose_ticks=2,cycle_ticks=64,each_pose_exactly_one_shadow=True,source_shadow_coverage=stats,total_expected=sum(r['expected_shadow_pixels']for r in stats),total_missing=sum(r['missing']for r in stats),limitations=['Conservative source-ground shadow subset; near-foot black is explicitly inferred contact paint.','Native black is source paint, not a reserved-key claim.','Five scene instances and native canopy compositing are not yet bound to this proof.']))
 print(dst)

if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
