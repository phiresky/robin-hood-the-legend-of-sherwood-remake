"""Reuse exact prior fork cameras and parent images for subsequent private trials."""
import sys,json
from pathlib import Path
import bpy
from mathutils import Matrix
from PIL import Image,ImageDraw
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
from restart6_tree24_contour import ROOT
from render_views import render_views
from evidence_io import sha,write_json
from render_slots import acquire,release
acquire()
try:
 version=int(sys.argv[sys.argv.index('--')+1]);model=ROOT/f'tree24-fork-union-v{version}/model.blend';prior=ROOT/'tree24-fork-delta-v4';out=ROOT/f'tree24-fork-delta-v{version}';out.mkdir(exist_ok=False);e=json.loads((prior/'evidence.json').read_text());bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene
 for o in scene.objects:
  if o.type=='MESH':o.hide_render=o.get('asset_group')!='croisement02-tree-24'or'Crown'in o.name
 scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=512;scene.render.film_transparent=True;scene.view_settings.view_transform='Standard';scene.view_settings.look='None';scene.render.resolution_x=512;scene.render.resolution_y=512;scene.display.shading.light='STUDIO';scene.display.shading.color_type='SINGLE';scene.display.shading.single_color=(.65,.65,.65);scene.display.shading.show_shadows=True;scene.display.shading.show_cavity=True;scene.display.shading.cavity_type='BOTH';views={}
 for i,info in enumerate(e['same_cameras']):
  cam=bpy.data.objects.new(f'Native-first fork {i}',bpy.data.cameras.new(f'Fork {i}'));scene.collection.objects.link(cam);cam.data.type='ORTHO';cam.data.ortho_scale=info['scale'];cam.data.clip_end=20000;cam.matrix_world=Matrix(info['matrix']);views[f'view-{i}']=cam.name
 render_views(scene.name,views,out/'candidate',modes=('solid','textured'),width=512)
 for mode,name in [('solid','comparison.png'),('textured','appearance-comparison.png')]:
  im=Image.new('RGB',(2048,1060),(28,28,28));draw=ImageDraw.Draw(im)
  for row,(label,folder)in enumerate([('Approved parent',prior/'parent'),('Private candidate',out/'candidate')]):
   draw.text((6,row*530+4),label+' original camera first, identical local framing',fill='white')
   for i in range(4):im.paste(Image.open(folder/f'view-{i}-{mode}.png').convert('RGB'),(i*512,row*530+18))
  im.save(out/name)
 write_json(out/'evidence.json',dict(candidate_sha256=sha(model),parent_sha256=e['parent_sha256'],same_cameras=e['same_cameras'],parent_images={str(p):sha(p)for p in(prior/'parent').glob('*.png')}))
finally:release()
