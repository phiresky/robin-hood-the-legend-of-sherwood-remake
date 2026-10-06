"""Inspect saved hidden-bark continuation, retaining native-first presentation."""
import os,sys,json
from pathlib import Path
import bpy
from mathutils import Vector
from PIL import Image
P=Path(__file__).resolve().parent;sys.path.insert(0,str(P));STAGE=sys.argv[sys.argv.index('--')+2] if len(sys.argv)>sys.argv.index('--')+2 else 'unseen-complete-v2';os.environ['C02_TOE_BARK_STAGE']=STAGE
from restart8_bake_toe_bark import B,read,h
from render_multiview_asset import render
from restart4_stump_final_contact import frame,sheet
from restart6_source_gap_audit import RAY
from render_slots import acquire,release
import restart8_review_toe_bark as shader
import restart8_toe_bark_native_guard as native
import restart8_toe_bark_contact as contact

def main(n):
 F=B/f'tree-{n}-fill-v1';D=F/STAGE;c=read(D/'continuation.json');assert h(D/'worker.blend')==c['model_sha256'];proof=read(F/'shader-restored-v1/preservation.json');proof.update(model_sha256=c['model_sha256'],continuation_sha256=h(D/'continuation.json'),status='Saved same-object hidden continuation; review pending');(D/'preservation.json').write_text(json.dumps(proof,indent=2)+'\n');bpy.ops.wm.open_mainfile(filepath=str(D/'worker.blend'));scene=bpy.context.scene;scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=512
 for scope in ['close','full']:
  hidden={o.name:o.hide_render for o in scene.objects}
  if scope=='close':
   for ob in scene.objects:
    if ob.type=='MESH'and ob.name not in proof['active_receivers']:ob.hide_render=True
  render(D/f'{scope}-views.json',D/scope,width=384);sheet([D/scope/f'view-{i}-textured.png'for i in range(8)],D/scope/'textured.png')
  for ob in scene.objects:ob.hide_render=hidden[ob.name]
 shader.main(n);native.main(n);contact.main(n)
 bpy.ops.wm.open_mainfile(filepath=str(D/'worker.blend'));scene=bpy.context.scene;wood=[o for o in scene.objects if o.type=='MESH'and o.name in proof['active_receivers']]
 for o in scene.objects:
  if o.type=='MESH':o.hide_render=o not in wood
 points=[o.matrix_world@v.co for o in wood for v in o.data.vertices if(o.matrix_world@v.co).z<(43 if n==19 else 71)];mesh=bpy.data.meshes.new('Lower framing diagnostic');mesh.from_pydata(points,[],[]);proxy=bpy.data.objects.new(mesh.name,mesh);scene.collection.objects.link(proxy);proxy.hide_render=True;U=D/'underside';U.mkdir();scene.render.engine='CYCLES';scene.cycles.samples=8
 for i,d in enumerate([RAY,Vector((.612,-.612,-.5)),Vector((.612,.612,-.5)),Vector((0,0,-1))]):
  frame(scene,[proxy],d.normalized(),384,1.4);scene.render.filepath=str(U/f'view-{i}.png');bpy.ops.render.render(write_still=True)
 sheet([U/f'view-{i}.png'for i in range(4)],U/'sheet.png')
 print('Complete saved review',n,c['model_sha256'],flush=True)
if __name__=='__main__':
 acquire()
 try:main(int(sys.argv[sys.argv.index('--')+1]))
 finally:release()
