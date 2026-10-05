"""Native-first saved hay contact against the separately reopened approved ground."""
import json,sys,math
from pathlib import Path
import bpy
from mathutils import Vector
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from tree_geometry import SIN,RAY
from render_slots import acquire,release

def main():
 w=OUT/'restart3-hay/candidate-v1/assets/croisement02-south-field-haystack';g=OUT/'restart2-ground-completion/approved-fill-retry-v2/bake-v1/model.blend';gh='16c638be71eeb76e86439a0fdb14bac1e7bb9562afe20d175b58d0df96fb4ec2';dest=OUT/'restart3-hay/ground-v1';dest.mkdir(exist_ok=False)
 if sha(g)!=gh:raise ValueError('Ground changed')
 digest=sha(w/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(g));bpy.context.view_layer.update();ground=[o for o in bpy.data.objects if o.type=='MESH' and o.get('source_node')=='ground'];expected={o.name:o.matrix_world.copy() for o in ground}
 if not ground:raise ValueError('Missing approved ground')
 bpy.ops.wm.open_mainfile(filepath=str(w/'model.blend'));bpy.context.view_layer.update();scene=bpy.data.scenes.new('Hay ground contact');bpy.context.window.scene=scene;hay=[]
 for o in list(bpy.data.collections['Croisement02 Working'].all_objects):
  if o.type=='MESH' and o.get('asset_group')==w.name:
   clone=o.copy();matrix=o.matrix_world.copy();clone.parent=None;scene.collection.objects.link(clone);clone.matrix_world=matrix;clone.hide_render=False;hay.append(clone)
 with bpy.data.libraries.load(str(g),link=False) as (_,loaded):loaded.objects=list(expected)
 restored=[]
 for name,o in zip(expected,loaded.objects):
  if o is None:raise ValueError('Missing imported ground')
  o.parent=None;scene.collection.objects.link(o);o.matrix_world=expected[name];o.hide_render=False;restored.append(o)
 bpy.context.view_layer.update()
 for name,o in zip(expected,restored):
  if max(abs(o.matrix_world[r][c]-expected[name][r][c]) for r in range(4) for c in range(4))>1e-5:raise ValueError('Ground world transform changed')
 scene.world=bpy.data.worlds.new('Hay ground neutral');scene.world.color=(.1,.1,.1);scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=64;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.render.film_transparent=True;scene.view_settings.view_transform='Standard';scene.view_settings.look='None';scene.render.resolution_percentage=100
 data=bpy.data.lights.new('Hay diagnostic sun','SUN');data.energy=2;light=bpy.data.objects.new(data.name,data);scene.collection.objects.link(light);light.rotation_euler=(.5,-.4,-.5);camdata=bpy.data.cameras.new('Hay native camera');camdata.type='ORTHO';camdata.sensor_fit='HORIZONTAL';camdata.clip_end=20000;cam=bpy.data.objects.new(camdata.name,camdata);scene.collection.objects.link(cam);scene.camera=cam;cameras=[]
 def render(name,target,direction,scale,width,height):
  cam.location=target+direction*5000;cam.rotation_euler=(target-cam.location).to_track_quat('-Z','Y').to_euler();camdata.ortho_scale=scale;scene.render.resolution_x=width;scene.render.resolution_y=height;scene.render.filepath=str(dest/name);bpy.context.view_layer.update();bpy.ops.render.render(write_still=True,scene=scene.name);cameras.append(dict(image=name,matrix=[list(r) for r in cam.matrix_world],ortho_scale=scale))
 box=(860,930,1050,1110);render('native.png',Vector((955,-1020/SIN,0)),RAY,190,190,180);source=Image.open(OUT/'animation-references/composite-frame-0.png').crop(box).convert('RGBA');actual=Image.open(dest/'native.png').convert('RGBA');board=Image.new('RGBA',(380,180),(80,80,80,255));board.alpha_composite(source,(0,0));board.alpha_composite(actual,(190,0));board.resize((1140,540),Image.Resampling.NEAREST).convert('RGB').save(dest/'source-comparison.png');sheet=Image.new('RGB',(2048,384),'#454545')
 for i in range(4):
  angle=i*math.tau/4;direction=RAY if i==0 else Vector((math.sin(angle)*math.cos(.35),-math.cos(angle)*math.cos(.35),math.sin(.35)));render(f'contact-{i}.png',Vector((959,-1796,26)),direction,230,512,384);im=Image.open(dest/f'contact-{i}.png').convert('RGBA');sheet.paste(im,(i*512,0),im)
 sheet.save(dest/'contact-sheet.png');write_json(dest/'evidence.json',dict(model_sha256=digest,ground_model=str(g),ground_sha256=gh,ground_transforms_verified_against_reopened_source=True,native_first=True,cameras=cameras,minimum_hay_z=min((o.matrix_world@v.co).z for o in hay for v in o.data.vertices),source_crop=list(box),files={p.name:sha(p) for p in dest.glob('*.png')}))
 if sha(w/'model.blend')!=digest or sha(g)!=gh:raise ValueError('Read-only review changed source')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
