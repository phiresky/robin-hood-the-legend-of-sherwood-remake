"""Saved Tree01 original-camera, solid and adjacent-bank review; private only."""
import sys,json,math,hashlib
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from PIL import Image,ImageDraw
sys.path.insert(0,str(Path(__file__).resolve().parent))
import restart2_tree01_canopy_saved as common
R=common.R;B=common.B;I=B/'tree01-static-crown-saved-v3';BANK=B/'bank-shoulder-lofts-saved-v4';O=I/'context-review-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 common.resources();common.acquire(slots=2)
 try:
  common.resources();assert not O.exists();O.mkdir();model=I/'worker.blend';pins={str(p):sha(p)for p in [model,BANK/'worker.blend']};bpy.ops.wm.open_mainfile(filepath=str(model));s=bpy.context.scene
  tree=[o for o in s.objects if o.type=='MESH' and (o.name.startswith('Tree01 ') or o.name.startswith('Provisional Tree01 wood'))]
  for o in s.objects:
   if o.type=='MESH':o.hide_render=o not in tree
  s.render.engine='CYCLES';s.cycles.samples=16;s.cycles.transparent_max_bounces=256;s.cycles.use_denoising=False;s.cycles.seed=17;s.render.threads_mode='FIXED';s.render.threads=2;s.render.resolution_percentage=100;s.render.film_transparent=True;s.render.image_settings.file_format='PNG';s.render.image_settings.color_mode='RGBA';s.view_settings.view_transform='Standard'
  camera=bpy.data.objects.new('Scoped review camera',bpy.data.cameras.new('Scoped review camera'));s.collection.objects.link(camera);camera.data.type='ORTHO';camera.data.sensor_fit='HORIZONTAL';camera.data.clip_end=10000;s.camera=camera
  points=np.array([list(o.matrix_world@v.co)for o in tree for v in o.data.vertices]);center=Vector((points.min(0)+points.max(0))/2)
  def render(name,direction,target,width,height,scale):
   camera.location=target+direction*5000;camera.rotation_euler=(-direction).to_track_quat('-Z','Y').to_euler();camera.data.ortho_scale=scale;s.render.resolution_x=width;s.render.resolution_y=height;s.render.filepath=str(O/name);bpy.ops.render.render(write_still=True);return Image.open(O/name).convert('RGBA')
  ray=Vector((0,-common.C,common.S));nativecenter=Vector((85,-105/common.S,0));native=render('native.png',ray,nativecenter,260,430,260)
  source=Image.open(B.parent/'baseline/covered.png').convert('RGBA');pad=Image.new('RGBA',(260,430),'#333333');pad.paste(source.crop((0,0,215,320)),(45,110));comparison=Image.new('RGB',(520,450),'#333333');comparison.paste(pad.convert('RGB'),(0,20));bg=Image.new('RGBA',native.size,'#333333');bg.alpha_composite(native);comparison.paste(bg.convert('RGB'),(260,20));draw=ImageDraw.Draw(comparison);draw.text((3,3),'Native source; outside-map gray',fill='white');draw.text((263,3),'Saved Tree01; inferred crown complete',fill='white');comparison.save(O/'native-comparison.png')
  gray=bpy.data.materials.new('Scoped solid review');gray.diffuse_color=(.4,.4,.4,1);s.view_layers[0].material_override=gray;sheet=Image.new('RGB',(1024,640),'#333333');scale=max(float(np.ptp(points[:,i]))for i in range(3))*.92
  for i in range(8):
   a=math.radians(i*45);im=render(f'solid-{i*45:03}.png',Vector((math.sin(a)*common.C,-math.cos(a)*common.C,common.S)),center,256,320,scale);bg=Image.new('RGBA',im.size,'#333333');bg.alpha_composite(im);sheet.paste(bg.convert('RGB'),(i%4*256,i//4*320))
  sheet.save(O/'solid-eight.png');s.view_layers[0].material_override=None
  with bpy.data.libraries.load(str(BANK/'worker.blend'),link=False)as(src,dst):dst.objects=[n for n in src.objects if n.startswith('Candidate bank')]
  assert len(dst.objects)==3,[o.name for o in dst.objects]
  for o in dst.objects:s.collection.objects.link(o);o.hide_render=False
  s.view_layers[0].update();render('native-bank-context.png',ray,Vector((300,-150/common.S,0)),700,600,700)
  for p,h in pins.items():assert sha(Path(p))==h
  receipt={'status':'SAVED_REVIEW_PENDING_VISUAL_CONTACT_AUDIT','pins':pins,'native_pixel_box':[-45,-110,215,320],'native_direction':list(ray),'tree_objects':[o.name for o in tree],'bank_objects':[o.name for o in dst.objects],'images':{p.name:sha(p)for p in O.glob('*.png')},'limits':['Bank context contains held bank candidate; no publication or approval implied.','Neighbor trees and native shared animation clock are not included.','Solid material displays foliage geometry without source cutout alpha.']};(O/'receipt.json').write_text(json.dumps(receipt,indent=2)+'\n');assert sum(p.stat().st_size for p in O.rglob('*')if p.is_file())<32*1024**2
 finally:common.release()
if __name__=='__main__':main()
