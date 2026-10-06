"""Review saved final root exteriors against the unchanged approved parent views."""
import sys,json
from pathlib import Path
import bpy
from mathutils import Matrix
from PIL import Image,ImageDraw
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
from restart6_source_gap_audit import ROOT
from render_views import render_views
from render_slots import acquire,release
from evidence_io import sha,write_json
acquire()
try:
 for tree,version in [(18,3),(39,2)]:
  model=ROOT/f'tree{tree}-continuous-finished-v{version}/model.blend';old=ROOT/f'tree{tree}-continuous-solid-delta-v1';info=json.load(open(old/'evidence.json'));out=model.parent/'lower-solid-review';out.mkdir(exist_ok=False);bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene
  for o in scene.objects:
   if o.type=='MESH':o.hide_render=o.get('asset_group')!=f'croisement02-tree-{tree}'or'Crown'in o.name
  views={}
  for i,caminfo in enumerate(info['same_cameras']):
   camera=bpy.data.objects.new(str(i),bpy.data.cameras.new(str(i)));scene.collection.objects.link(camera);camera.data.type='ORTHO';camera.data.ortho_scale=caminfo['scale'];camera.data.clip_end=20000;camera.matrix_world=Matrix(caminfo['matrix']);views[f'view-{i}']=camera.name
  scene.render.resolution_x=512;scene.render.resolution_y=512;scene.display.shading.light='STUDIO';scene.display.shading.color_type='SINGLE';scene.display.shading.single_color=(.65,.65,.65);scene.display.shading.show_shadows=True;scene.display.shading.show_cavity=True;scene.display.shading.cavity_type='BOTH';render_views(scene.name,views,out,modes=('solid',),width=512);im=Image.new('RGB',(2048,1060),(28,28,28));draw=ImageDraw.Draw(im)
  for row,label in enumerate(['Approved parent','Final continuous exterior']):
   draw.text((6,row*530+4),label+'; original camera first, identical framing',fill='white')
   for i in range(4):im.paste(Image.open((old/'parent'if row==0 else out)/f'view-{i}-solid.png').convert('RGB'),(i*512,row*530+18))
  im.save(out/'comparison.png');write_json(out/'evidence.json',dict(model_sha256=sha(model),parent_sha256=info['parent_sha256'],parent_evidence_sha256=sha(old/'evidence.json'),cameras=info['same_cameras'],scope='Saved final geometry, unchanged approved parent render reused with identical cameras.'))
finally:release()
