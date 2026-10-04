"""Inspect the private corrected wattle fence beside the selected flowering clumps."""
import json,sys,math
from pathlib import Path
import bpy
from PIL import Image,ImageDraw
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT,scenery_workspace
from tree_geometry import SIN,COS,RAY
from evidence_io import sha,write_json
from render_slots import acquire,release
from stage_review_scene import signature

def main():
 base=OUT/'wattle99-source-candidate/v6';model=base/'model.blend';digest=sha(model);assert digest==json.loads((base/'reopened-review/validation.json').read_text())['model_sha256'];plant=scenery_workspace('croisement02-shrub-76');plant_hash=sha(plant/'model.blend');audit=json.loads((plant/'inspection/saved-model-audit.json').read_text());assert audit['status']=='PASS' and audit['model_sha256']==plant_hash;dst=base/'flower-joint-v2';dst.mkdir(exist_ok=False);bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;asset='croisement02-southwest-path-wattle-fence';selected=[o for o in scene.objects if o.type=='MESH' and o.get('asset_group')==asset];assert len(selected)==1;fence=selected[0]
 for obj in list(scene.objects):
  if obj.type=='MESH'and obj!=fence:bpy.data.objects.remove(obj,do_unlink=True)
 with bpy.data.libraries.load(str(plant/'model.blend'),link=False)as(src,data):data.objects=[r['object']for r in audit['objects']]
 plants=[]
 for obj in data.objects:
  scene.collection.objects.link(obj);before=signature(obj);world=obj.matrix_world.copy();obj.parent=None;obj.matrix_world=world;obj.hide_render=False;assert signature(obj)==before;plants.append(obj)
 scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=128;scene.cycles.use_denoising=False;scene.render.film_transparent=True;scene.render.image_settings.color_mode='RGBA';data=bpy.data.cameras.new('Source and oblique flower joint');data.type='ORTHO';data.sensor_fit='HORIZONTAL';data.clip_end=20000;camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera);scene.camera=camera;crop=(526,741,792,1026);l,t,r,b=crop;target=Vector(((l+r)/2,-(t+b)/2/SIN,0));data.ortho_scale=r-l;camera.location=target+RAY*5000;camera.rotation_euler=(target-camera.location).to_track_quat('-Z','Y').to_euler();scene.render.resolution_x=(r-l)*3;scene.render.resolution_y=(b-t)*3;scene.render.resolution_percentage=100;scene.render.filepath=str(dst/'source-joint.png');bpy.ops.render.render(write_still=True);source=Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA').crop(crop).resize((scene.render.resolution_x,scene.render.resolution_y),Image.Resampling.NEAREST);actual=Image.open(scene.render.filepath).convert('RGBA');comparison=Image.new('RGB',(source.width*2,source.height),(70,70,70));comparison.paste(source.convert('RGB'));comparison.paste(actual,(source.width,0),actual.getchannel('A'));comparison.save(dst/'source-comparison.png');saved=list(fence.data.materials);mat=bpy.data.materials.new('Fence first-hit diagnostic magenta');mat.use_nodes=True;nodes=mat.node_tree.nodes;nodes.clear();emit=nodes.new('ShaderNodeEmission');emit.inputs[0].default_value=(1,0,1,1);out=nodes.new('ShaderNodeOutputMaterial');mat.node_tree.links.new(emit.outputs[0],out.inputs[0])
 for i in range(len(fence.data.materials)):fence.data.materials[i]=mat
 scene.render.filepath=str(dst/'first-hit-fence.png');bpy.ops.render.render(write_still=True);fence.hide_render=True;scene.render.filepath=str(dst/'plants-only.png');bpy.ops.render.render(write_still=True);fence.hide_render=False
 for i,material in enumerate(saved):fence.data.materials[i]=material
 import numpy as np
 rgba=np.asarray(Image.open(dst/'first-hit-fence.png').convert('RGBA'))[1::3,1::3];plant_alpha=np.asarray(Image.open(dst/'plants-only.png').convert('RGBA'))[1::3,1::3,3]>127;known=np.asarray(Image.open(OUT/'mixed-wood-audit/domain-502.png').convert('L'))[t:b,l:r]>127;blocked=(rgba[:,:,0]>245)&(rgba[:,:,1]<10)&(rgba[:,:,2]>245)&(rgba[:,:,3]>127);coverage=dict(observed_leaf_pixels=int(known.sum()),physical_leaf_alone_coverage=int((known&plant_alpha).sum()),observed_leaf_blocked_by_fence=int((known&plant_alpha&blocked).sum()));write_json(dst/'first-hit.json',coverage);points=[o.matrix_world@v.co for o in [fence,*plants]for v in o.data.vertices];center=Vector([(min(p[i]for p in points)+max(p[i]for p in points))/2 for i in range(3)]);sheet=Image.new('RGB',(1536,768),(70,70,70));scene.render.resolution_x=384;scene.render.resolution_y=384
 for i in range(8):
  angle=i*math.pi/4;direction=Vector((math.sin(angle)*COS,-math.cos(angle)*COS,SIN));camera.location=center+direction*5000;camera.rotation_euler=(center-camera.location).to_track_quat('-Z','Y').to_euler();bpy.context.view_layer.update();local=[camera.matrix_world.inverted()@p for p in points];data.ortho_scale=max(max(p[j]for p in local)-min(p[j]for p in local)for j in [0,1])*1.12;scene.render.filepath=str(dst/f'view-{i}.png');bpy.ops.render.render(write_still=True);im=Image.open(scene.render.filepath).convert('RGBA');sheet.paste(im,((i%4)*384,(i//4)*384),im.getchannel('A'))
 sheet.save(dst/'actual-eight.png');assert sha(model)==digest and sha(plant/'model.blend')==plant_hash;write_json(dst/'evidence.json',dict(status='Private physical flower/fence joint; visual and source-boundary review pending',fence_model_sha256=digest,plant_model_sha256=plant_hash,plant_workspace=str(plant),source_crop=crop,limitations=['Canopy42/129 and other physical neighbors are absent; their excluded source pixels remain gray on this source-only fence.','Remaining22 inferred foliage76 edge pixels are not silently assigned to current observed502 plant geometry.','No canonical model or source-domain receipt is changed.']));print(dst)

if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
