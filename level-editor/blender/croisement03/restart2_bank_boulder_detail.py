"""Render saved whole rock masses closely in eight directions without changing geometry."""
import sys,math,json
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
sys.path.insert(0,str(Path(__file__).resolve().parent))
import restart2_bank_face_source_saved as source
import restart2_bank_full_v1 as base

def main():
 source.guard.resources();source.guard.acquire(slots=2)
 try:
  source.guard.resources();folder=source.OUT;out=folder/'boulder-detail-v1';assert not out.exists();out.mkdir();model=folder/'worker.blend';digest=source.sha(model);bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;allpoints=[o.matrix_world@v.co for o in scene.objects if o.type=='MESH'for v in o.data.vertices];receipts={}
  for label,number in [('east',52),('south',54)]:
   obj=scene.objects[f'Candidate bank {number}'];points=[obj.matrix_world@v.co for v in obj.data.vertices if number==54 or (obj.matrix_world@v.co).x>=548];assert points;points_array=np.array(points);target=Vector((points_array.min(0)+points_array.max(0))/2);views={};scales=[]
   for i in range(8):
    a=i*math.tau/8;direction=Vector((math.sin(a)*base.C,-math.cos(a)*base.C,base.S));cam=bpy.data.objects.new(f'{label} mass {i}',bpy.data.cameras.new(f'{label} mass {i}'));scene.collection.objects.link(cam);cam.data.type='ORTHO';cam.location=target+direction*5000;cam.rotation_euler=(-direction).to_track_quat('-Z','Y').to_euler();scene.view_layers[0].update();inv=cam.matrix_world.inverted();local=[inv@p for p in points];scales.append(2*max(max(abs(p.x),abs(p.y))for p in local)*1.1);depth=[-(inv@p).z for p in allpoints];cam.data.clip_start=min(depth)-25;cam.data.clip_end=max(depth)+25;views[f'view-{i}']=cam.name
   for name in views.values():scene.objects[name].data.ortho_scale=max(scales)
   base.render_views(scene.name,views,out/label,modes=('textured','solid'),width=256)
   for mode in ['textured','solid']:
    sheet=Image.new('RGB',(1024,512),'#333333')
    for i in range(8):
     im=Image.open(out/label/f'view-{i}-{mode}.png').convert('RGBA');bg=Image.new('RGBA',im.size,'#333333');bg.alpha_composite(im);sheet.paste(bg.convert('RGB'),(i%4*256,i//4*256))
    sheet.save(out/f'{label}-{mode}8.png')
   receipts[label]={'target_vertices':len(points),'orthographic_scale':max(scales),'native_view_index':0}
  assert source.sha(model)==digest;assert sum(p.stat().st_size for p in folder.rglob('*')if p.is_file())<32*1024**2;(out/'receipt.json').write_text(json.dumps({'model_sha256':digest,'framing':receipts,'limits':['Full saved model rendered; camera focuses selected stone masses.','Other bank surfaces remain surrounding context, no geometry hidden or changed.']},indent=2)+'\n')
 finally:source.guard.release()
if __name__=='__main__':main()
