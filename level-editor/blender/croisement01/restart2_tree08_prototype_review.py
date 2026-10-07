"""Small native-first solid review of saved Tree08 scaffold, without model mutation."""
import hashlib,json,math,shutil,sys
from pathlib import Path
import bpy
from mathutils import Vector
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
R=ROOT/'level-editor/work/croisement01-refinement/restart2';p=R/'tree08-wood-prototype-v3';out=p/'solid8-v2';floor=10*1024**3;budget=128*1024**2

def guard(reserve=2*1024**2):
 used=sum(f.stat().st_size for d in R.glob('tree08-wood-prototype-v*') for f in d.rglob('*') if f.is_file());assert used+reserve<budget;assert shutil.disk_usage(R).free>=floor+budget-used

guard();out.mkdir(exist_ok=True);assert not any(out.iterdir()),'Do not overwrite evidence';acquire();bpy.ops.wm.open_mainfile(filepath=str(p/'model.blend'));scene=bpy.context.scene;scene.render.threads_mode='FIXED';scene.render.threads=2;scene.render.engine='BLENDER_WORKBENCH';scene.display.shading.light='STUDIO';scene.display.shading.color_type='SINGLE';scene.display.shading.single_color=(.5,.5,.5);scene.display.shading.show_shadows=True;scene.display.shading.show_cavity=True;scene.render.resolution_x=384;scene.render.resolution_y=384;scene.render.resolution_percentage=100;scene.render.image_settings.file_format='PNG';scene.world=bpy.data.worlds.new('Diagnostic neutral world');scene.world.color=(.1,.1,.1)
s,c=math.sin(math.radians(35)),math.cos(math.radians(35));ray=Vector((0,-c,s));down=Vector((0,-s,-c));camdata=bpy.data.cameras.new('Native first camera');cam=bpy.data.objects.new('Native first camera',camdata);scene.collection.objects.link(cam);camdata.type='ORTHO';camdata.clip_end=10000;camdata.ortho_scale=535;scene.camera=cam
obj=next(o for o in scene.objects if o.type=='MESH');points=[obj.matrix_world@v.co for v in obj.data.vertices];target=Vector(tuple((min(v[k] for v in points)+max(v[k] for v in points))/2 for k in range(3)))
for i in range(8):
 a=math.tau*i/8;direction=Vector((c*math.sin(a),-c*math.cos(a),s));cam.location=target+direction*1800;cam.rotation_euler=(-direction).to_track_quat('-Z','Y').to_euler();scene.render.filepath=str(out/f'{i}.png');guard();bpy.ops.render.render(write_still=True)
guard();sheet=Image.new('RGB',(1536,816),'#222222');draw=ImageDraw.Draw(sheet)
for i in range(8):
 x=i%4*384;y=i//4*408;sheet.paste(Image.open(out/f'{i}.png').convert('RGB'),(x,y+24));draw.text((x+5,y+5),'0 - Original game camera' if i==0 else f'{i} - Solid construction',fill='white')
sheet.save(out/'sheet.png');guard();(out/'receipt.json').write_text(json.dumps(dict(model_sha256=hashlib.sha256((p/'model.blend').read_bytes()).hexdigest(),native_camera_first=True,model_unchanged=True,scope='Solid shape only; separate swept sections, no terrain/contact or final source ownership proof'),indent=2)+'\n')
