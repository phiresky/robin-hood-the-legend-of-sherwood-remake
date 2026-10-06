"""Compare approved and proposed lower-root construction in identical cameras."""
import sys,json
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector,Matrix
from PIL import Image,ImageDraw
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
from restart6_source_gap_audit import ROOT,SPECS,RAY
from restart4_stump_final_contact import frame,sheet
from render_views import render_views
from render_slots import acquire,release
from evidence_io import sha,write_json
def own():return [o for o in bpy.context.scene.objects if o.type=='MESH'and o.get('asset_group')=='croisement02-tree-39']
def main():
 out=ROOT/'tree39-solid-delta-v1';out.mkdir(exist_ok=False);parent=SPECS[39][0];candidate=ROOT/'tree39-contour-v6/model.blend';bpy.ops.wm.open_mainfile(filepath=str(parent));bpy.context.view_layer.update();before={o.name:np.array([o.matrix_world@v.co for v in o.data.vertices])for o in own()};bpy.ops.wm.open_mainfile(filepath=str(candidate));bpy.context.view_layer.update();changed=[];deltas={}
 for o in own():
  p=np.array([o.matrix_world@v.co for v in o.data.vertices]);d=np.linalg.norm(p-before[o.name],axis=1);changed.extend(p[d>1e-5]);deltas[o.name]=dict(changed=int((d>1e-5).sum()),max=float(d.max()))
 mesh=bpy.data.meshes.new('Delta framing');mesh.from_pydata(changed,[],[]);proxy=bpy.data.objects.new(mesh.name,mesh);bpy.context.scene.collection.objects.link(proxy);cameras=[]
 for d in [RAY,Vector((.6124,-.6124,.5)),Vector((.6124,.6124,.5)),Vector((-.866,0,.5))]:
  cam=frame(bpy.context.scene,[proxy],d.normalized(),512,1.25);cameras.append(dict(matrix=[list(r)for r in cam.matrix_world],scale=cam.data.ortho_scale))
 for label,path in [('parent',parent),('candidate',candidate)]:
  bpy.ops.wm.open_mainfile(filepath=str(path));scene=bpy.context.scene
  for o in scene.objects:
   if o.type=='MESH':o.hide_render=o.get('asset_group')!='croisement02-tree-39'or'Crown'in o.name
  scene.render.resolution_x=512;scene.render.resolution_y=512;scene.display.shading.light='STUDIO';scene.display.shading.color_type='SINGLE';scene.display.shading.single_color=(.65,.65,.65);scene.display.shading.show_shadows=True;scene.display.shading.show_cavity=True;scene.display.shading.cavity_type='BOTH';views={}
  for i,info in enumerate(cameras):
   cam=bpy.data.objects.new(f'{label} native-first {i}',bpy.data.cameras.new(f'{label} {i}'));scene.collection.objects.link(cam);cam.data.type='ORTHO';cam.data.ortho_scale=info['scale'];cam.data.clip_end=20000;cam.matrix_world=Matrix(info['matrix']);views[f'view-{i}']=cam.name
  render_views(scene.name,views,out/label,modes=('solid',),width=512)
 im=Image.new('RGB',(2048,1060),(28,28,28));draw=ImageDraw.Draw(im)
 for row,label in enumerate(['parent','candidate']):
  draw.text((6,row*530+4),f'{label}: original camera first; identical lower-root framing',fill='white')
  for i in range(4):im.paste(Image.open(out/label/f'view-{i}-solid.png').convert('RGB'),(i*512,row*530+18))
 im.save(out/'comparison.png');write_json(out/'evidence.json',dict(parent_sha256=sha(parent),candidate_sha256=sha(candidate),same_cameras=cameras,movement=deltas,scope='Read-only solid shape comparison, crown hidden. No model changes.'))
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
