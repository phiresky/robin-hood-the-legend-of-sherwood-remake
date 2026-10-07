"""Two deliberately cropped contact views of the unchanged shared-ridge candidate."""
import sys,math,hashlib,json
from pathlib import Path
import bpy
from mathutils import Vector
from PIL import Image,ImageDraw
R=Path(__file__).resolve().parents[3];sys.path[:0]=[str(R/'level-editor/refinement'),str(R/'level-editor/refinement/blender')]
from render_views import render_views as render
from evidence_io import sha,write_json
B=R/'level-editor/work/croisement03-refinement/restart2';BASE=R/'level-editor/blender/croisement03/restart2_tree02_shared_ridge_joint_v5.py'
def contact(scene_name,views,output,**ignored):
 s=bpy.data.scenes[scene_name];sn=math.sin(math.radians(35));cs=math.cos(math.radians(35));target=Vector((270,-435,(435*sn-170)/cs));points=[o.matrix_world@v.co for o in s.objects if o.type=='MESH' for v in o.data.vertices];chosen={};bounds={}
 for i,label in [(0,'native'),(4,'reverse')]:
  cam=s.objects[f'Tree02 view{i}'];a=i*math.tau/8;direction=Vector((math.sin(a)*cs,-math.cos(a)*cs,sn));cam.location=target+direction*1500;cam.rotation_euler=(-direction).to_track_quat('-Z','Y').to_euler();cam.data.ortho_scale=180;bpy.context.view_layer.update();depths=[-(cam.matrix_world.inverted()@p).z for p in points];cam.data.clip_start=max(.1,min(depths)-25);cam.data.clip_end=max(depths)+25;chosen[label]=cam.name;bounds[label]=[cam.data.clip_start,cam.data.clip_end]
 render(scene_name,chosen,output,modes=('textured',),width=512)
 out=output.parent;src=Image.open(B.parent/'baseline/covered.png').convert('RGBA');ar=Image.open(B.parent/'animation-references/animation-07/000.png').convert('RGBA');src.alpha_composite(ar,(0,0));box=(180,80,360,260);original=src.crop(box).resize((512,512),Image.Resampling.NEAREST);sheet=Image.new('RGB',(1536,550),'#333333');d=ImageDraw.Draw(sheet)
 for i,(label,im) in enumerate([('Native art spatial context (frame0)',original),('Saved native camera contact',Image.open(output/'native-textured.png')),('Saved reverse contact',Image.open(output/'reverse-textured.png'))]):
  rgba=im.convert('RGBA');bg=Image.new('RGBA',rgba.size,'#333333');bg.alpha_composite(rgba);sheet.paste(bg.convert('RGB'),(i*512,30));d.text((i*512+5,10),label,fill='white')
 sheet.save(out/'contact-comparison.png');write_json(out/'contact-receipt.json',dict(status='Private contact self/root review required',base_recipe_sha256=sha(BASE),tree02_sha256=sha(B/'tree02-isolated-prototype-v7/worker.blend'),tree03_sha256=sha(B/'tree03-ridge-leaf-derivative-v2/worker.blend'),native_crop=list(box),native_first=True,depth_bounds=bounds,ortho_scale=180,sheet_sha256=sha(out/'contact-comparison.png'),limits=['Deliberately cropped contact details; complete full-frustum views are shared-ridge-joint-v5.','Original art panel is spatial covered-map plus Arbre08 frame0 comparison, not a full global-order/runtime parity proof.','Neutral receiver rear and unresolved surrounding painted ground are context only.']))
def main():
 code=BASE.read_text().replace("tree02-shared-ridge-joint-v5","tree02-shared-ridge-contact-v1")
 start=code.index('  render_views(s.name,views,');end=code.index('\n finally:release()',start)
 code=code[:start]+"  render_views(s.name,views,out/'views')"+code[end:]
 ns={'__file__':str(BASE),'__name__':'shared_ridge_private_contact'};exec(compile(code,str(BASE),'exec'),ns);ns['render_views']=contact;ns['main']()
if __name__=='__main__':main()
