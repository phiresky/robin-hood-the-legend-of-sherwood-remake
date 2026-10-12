"""Saved Tree01 original-camera, solid and adjacent-bank review; private only."""
import sys,json,math,hashlib,argparse
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from PIL import Image,ImageDraw
sys.path.insert(0,str(Path(__file__).resolve().parent))
import restart2_tree01_canopy_saved as common
R=common.R;B=common.B;I=B/'tree01-static-crown-saved-v3';BANK=B/'bank-shoulder-lofts-saved-v4';O=I/'context-review-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 global I,BANK,O
 ap=argparse.ArgumentParser();ap.add_argument('folder',type=Path,nargs='?',default=I);ap.add_argument('--bank',type=Path,default=BANK);ap.add_argument('--source-ground',action='store_true');ap.add_argument('--context-only',action='store_true');ap.add_argument('--review-dir',default='context-review-v1');args=ap.parse_args(sys.argv[sys.argv.index('--')+1:]if '--'in sys.argv else[]);I=args.folder.resolve();BANK=args.bank.resolve();O=I/args.review_dir;assert O.parent==I
 common.resources();common.acquire(slots=2)
 try:
  common.resources();assert not O.exists();O.mkdir();model=I/'worker.blend';pins={str(p):sha(p)for p in [model,BANK/'worker.blend']};ground_reference=None;ground_source=B.parent/'baseline/croisement03-baseline.blend'
  if args.source_ground:
   pins[str(ground_source)]=sha(ground_source);bpy.ops.wm.open_mainfile(filepath=str(ground_source));bpy.context.view_layer.update();g=bpy.data.objects['ground.001'];ground_reference={'matrix':np.array(g.matrix_world),'points':np.array([list(g.matrix_world@v.co)for v in g.data.vertices])}
  bpy.ops.wm.open_mainfile(filepath=str(model));s=bpy.context.scene
  tree=[o for o in s.objects if o.type=='MESH' and (o.name.startswith('Tree01 ') or o.name.startswith('Provisional Tree01 wood'))]
  for o in s.objects:
   if o.type=='MESH':o.hide_render=o not in tree
  s.render.engine='CYCLES';s.cycles.samples=16;s.cycles.transparent_max_bounces=256;s.cycles.use_denoising=False;s.cycles.seed=17;s.render.threads_mode='FIXED';s.render.threads=2;s.render.resolution_percentage=100;s.render.film_transparent=True;s.render.image_settings.file_format='PNG';s.render.image_settings.color_mode='RGBA';s.view_settings.view_transform='Standard'
  camera=bpy.data.objects.new('Scoped review camera',bpy.data.cameras.new('Scoped review camera'));s.collection.objects.link(camera);camera.data.type='ORTHO';camera.data.sensor_fit='HORIZONTAL';camera.data.clip_end=10000;s.camera=camera
  points=np.array([list(o.matrix_world@v.co)for o in tree for v in o.data.vertices]);center=Vector((points.min(0)+points.max(0))/2)
  def render(name,direction,target,width,height,scale):
   camera.location=target+direction*5000;camera.rotation_euler=(-direction).to_track_quat('-Z','Y').to_euler();camera.data.ortho_scale=scale;s.render.resolution_x=width;s.render.resolution_y=height;s.render.filepath=str(O/name);bpy.ops.render.render(write_still=True);return Image.open(O/name).convert('RGBA')
  ray=Vector((0,-common.C,common.S))
  if not args.context_only:
   nativecenter=Vector((85,-105/common.S,0));native=render('native.png',ray,nativecenter,260,430,260)
   source=Image.open(B.parent/'baseline/covered.png').convert('RGBA');source.alpha_composite(Image.open(B/'tree01-native-fragment-v1/000.png').convert('RGBA'));pad=Image.new('RGBA',(260,430),'#333333');pad.paste(source.crop((0,0,215,320)),(45,110));comparison=Image.new('RGB',(520,450),'#333333');comparison.paste(pad.convert('RGB'),(0,20));bg=Image.new('RGBA',native.size,'#333333');bg.alpha_composite(native);comparison.paste(bg.convert('RGB'),(260,20));draw=ImageDraw.Draw(comparison);draw.text((3,3),'Source context + native frame0',fill='white');draw.text((263,3),'Saved Tree01; inferred crown complete',fill='white');comparison.save(O/'native-comparison.png')
   gray=bpy.data.materials.new('Scoped solid review');gray.diffuse_color=(.4,.4,.4,1);s.view_layers[0].material_override=gray;sheet=Image.new('RGB',(1024,640),'#333333');scale=max(float(np.ptp(points[:,i]))for i in range(3))*.92
   for i in range(8):
    a=math.radians(i*45);im=render(f'solid-{i*45:03}.png',Vector((math.sin(a)*common.C,-math.cos(a)*common.C,common.S)),center,256,320,scale);bg=Image.new('RGBA',im.size,'#333333');bg.alpha_composite(im);sheet.paste(bg.convert('RGB'),(i%4*256,i//4*320))
   sheet.save(O/'solid-eight.png');s.view_layers[0].material_override=None
  with bpy.data.libraries.load(str(BANK/'worker.blend'),link=False)as(src,dst):dst.objects=[n for n in src.objects if n.startswith('Candidate bank')]
  assert len(dst.objects)==3,[o.name for o in dst.objects]
  for o in dst.objects:s.collection.objects.link(o);o.hide_render=False
  s.view_layers[0].update();ground_proof=None
  if args.source_ground:
   with bpy.data.libraries.load(str(ground_source),link=False)as(src,ground_data):ground_data.objects=['ground.001']
   ground=ground_data.objects[0];chain=[];current=ground
   while current is not None:chain.append(current);current=current.parent
   for current in reversed(chain):
    if current.name not in s.objects:s.collection.objects.link(current)
    current.hide_render=False
   s.view_layers[0].update();matrix_error=float(np.max(np.abs(np.array(ground.matrix_world)-ground_reference['matrix'])));points=np.array([list(ground.matrix_world@v.co)for v in ground.data.vertices]);point_error=float(np.max(np.abs(points-ground_reference['points'])));assert matrix_error<1e-6 and point_error<1e-5
   foot_heights=[float(min((o.matrix_world@v.co).z for v in o.data.vertices))for o in tree if o.name.startswith('Provisional Tree01 wood')];assert max(abs(z)for z in foot_heights)<1e-4;ground_proof={'matrix_error':matrix_error,'world_vertex_error':point_error,'wood_min_z':foot_heights,'ground_z_range':[float(points[:,2].min()),float(points[:,2].max())],'status':'Original ground geometry independently reopened and linked with parent transforms; artwork remains original context, receiver cleanup not yet applied.'}
  render('native-bank-context.png',ray,Vector((300,-175/common.S,0)),700,700,700)
  context_points=[o.matrix_world@v.co for o in [*tree,*dst.objects]for v in o.data.vertices];direction=Vector((common.C/2**.5,-common.C/2**.5,common.S));rotation=(-direction).to_track_quat('-Z','Y');origin=Vector(np.array([list(p)for p in context_points]).mean(0));local=np.array([list(rotation.conjugated()@(p-origin))for p in context_points]);center_xy=(local[:,:2].min(0)+local[:,:2].max(0))/2;target=origin+rotation@Vector((center_xy[0],center_xy[1],0));scale=max(float(np.ptp(local[:,0])),float(np.ptp(local[:,1])))*1.12
  render('oblique-bank-context.png',direction,target,700,700,scale)
  for p,h in pins.items():assert sha(Path(p))==h
  receipt={'status':'SAVED_REVIEW_PENDING_VISUAL_CONTACT_AUDIT','pins':pins,'ground_context':ground_proof,'native_pixel_box':[-45,-110,215,320],'native_direction':list(ray),'tree_objects':[o.name for o in tree],'bank_objects':[o.name for o in dst.objects],'images':{p.name:sha(p)for p in O.glob('*.png')},'limits':['Bank context contains held bank candidate; no publication or approval implied.','Neighbor trees and native shared animation clock are not included.','Solid material displays foliage geometry without source cutout alpha.']};(O/'receipt.json').write_text(json.dumps(receipt,indent=2)+'\n');assert sum(p.stat().st_size for p in O.rglob('*')if p.is_file())<32*1024**2
 finally:common.release()
if __name__=='__main__':main()
