"""Inspect saved per-pose sign materials from eight oblique camera directions."""
import json,sys,math
from pathlib import Path
import bpy
from PIL import Image
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from tree_geometry import SIN,COS
from evidence_io import sha,write_json
from render_slots import acquire,release

def main():
 base=OUT/'state-sign-candidate/phase-appearance-v1';source=base/'model.blend';digest=sha(source);assert digest==json.loads((base/'evidence.json').read_text())['model_sha256'];dst=base/'oblique-review';dst.mkdir(exist_ok=False);bpy.ops.wm.open_mainfile(filepath=str(source));scene=bpy.context.scene;data=bpy.data.cameras.new('Saved phase material inspection');data.type='ORTHO';data.ortho_scale=80;camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera);scene.camera=camera;scene.render.resolution_x=384;scene.render.resolution_y=384;scene.render.resolution_percentage=100;scene.cycles.samples=8;scene.cycles.use_denoising=False;scene.cycles.transparent_max_bounces=128;target=Vector((0,0,22));records=[]
 for phase in [0,8,16,24]:
  scene.frame_set(1+2*phase);sheet=Image.new('RGB',(1536,768),(70,70,70))
  for i in range(8):
   angle=math.radians(i*45);camera.location=target+Vector((math.sin(angle)*COS,-math.cos(angle)*COS,SIN))*500;camera.rotation_euler=(target-camera.location).to_track_quat('-Z','Y').to_euler();path=dst/f'phase-{phase:02}-view-{i}.png';scene.render.filepath=str(path);bpy.ops.render.render(write_still=True);render=Image.open(path).convert('RGBA');canvas=Image.new('RGB',render.size,(70,70,70));canvas.paste(render,mask=render.getchannel('A'));sheet.paste(canvas,((i%4)*384,(i//4)*384));records.append(dict(phase=phase,view=i,path=str(path),sha256=sha(path)))
  sheet.save(dst/f'phase-{phase:02}-actual-eight.png')
 assert sha(source)==digest;write_json(dst/'evidence.json',dict(status='Saved material inspection, visual review pending',model_sha256=digest,views=records,limitations=['Four native phases have all eight oblique views; all32 source-camera poses are retained in the parent packet.','This is an isolated reusable asset, not a full physical scene integration.']))
 print(dst)

if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
