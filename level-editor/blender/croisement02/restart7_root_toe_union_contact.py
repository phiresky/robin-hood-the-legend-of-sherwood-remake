"""Reopened native coverage and before/after lower contact evidence."""
import sys,json
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from PIL import Image
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
from restart6_source_gap_audit import ROOT,OUT,RAY,SIN
from restart6_tree38_contour import covered
from restart4_stump_final_contact import frame,sheet
from evidence_io import sha,write_json
from render_slots import acquire,release

def main(number):
 out=ROOT/f'tree{number}-toe-union-v7';model=out/'model.blend';prior=json.load(open(ROOT/f'baseline-audit-{number}-v3/report.json'));source=Path(prior['source']);m=next(x for x in json.load(open(OUT/'review-mask-inventory.json'))['masks']if x['index']==number);yy,xx=np.where(np.array(Image.open(m['png']))>0);targets=np.column_stack((xx+m['box_top_left'][0]+.5,yy+m['box_top_left'][1]+.5));results=[];cameras={};asset=f'croisement02-tree-{number}';limit=42 if number==19 else 70
 for tag,path in [('before',source),('after',model)]:
  bpy.ops.wm.open_mainfile(filepath=str(path));bpy.context.view_layer.update();scene=bpy.context.scene;own=[o for o in scene.objects if o.type=='MESH'and o.get('asset_group')==asset];wood=[o for o in own if o.get('projection_component')!='crown'];cov=covered(wood,targets);results.append(cov);points=[o.matrix_world@v.co for o in wood for v in o.data.vertices if (o.matrix_world@v.co).z<limit];mesh=bpy.data.meshes.new('Lower review framing');mesh.from_pydata(points,[],[]);proxy=bpy.data.objects.new(mesh.name,mesh);scene.collection.objects.link(proxy);proxy.hide_render=True
  for o in scene.objects:
   if o.type=='MESH':o.hide_render=o not in wood
  center=np.array(points).mean(0);bpy.ops.mesh.primitive_plane_add(size=180,location=(center[0],center[1],0));floor=bpy.context.object;floor.name='Z0 guide';mat=bpy.data.materials.new('Neutral ground');mat.diffuse_color=(.12,.14,.1,1);floor.data.materials.append(mat);scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=512;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';scene.view_settings.look='None';dirs=[RAY,Vector((.6124,-.6124,.5)),Vector((.6124,.6124,.5)),Vector((-.866,0,.5))]
  for i,d in enumerate(dirs):
   cam=frame(scene,[proxy],d.normalized(),384,1.4)
   if tag=='before':cameras[i]=(cam.matrix_world.copy(),cam.data.ortho_scale)
   else:cam.matrix_world=cameras[i][0];cam.data.ortho_scale=cameras[i][1]
   scene.render.filepath=str(out/f'{tag}-contact-{i}.png');bpy.ops.render.render(write_still=True)
  sheet([out/f'{tag}-contact-{i}.png'for i in range(4)],out/f'{tag}-contact4.png')
  from review_sunlight import render_solids
  cams=[]
  for i,d in enumerate(dirs):
   cam=frame(scene,[proxy],d.normalized(),384,1.4);cam.matrix_world=cameras[i][0];cam.data.ortho_scale=cameras[i][1];cams.append(cam)
  solid=out/f'{tag}-solid';solid.mkdir(exist_ok=True);render_solids(scene,cams,wood+[floor],solid,lighting={'toward_sun':[-.4511292577,-.5513802171,.7017565966],'ambient':.22,'diffuse':.78,'shadow_epsilon':.05});sheet([solid/f'view-{i}-solid.png'for i in range(4)],out/f'{tag}-solid-contact4.png')
 write_json(out/'native-coverage.json',dict(source_sha256=sha(source),model_sha256=sha(model),old_hits=int(results[0].sum()),new_hits=int(results[1].sum()),lost=(targets[results[0]&~results[1]]-.5).tolist(),gained=(targets[~results[0]&results[1]]-.5).tolist(),scope='Positive-height native center coverage only; not material/provenance/complete geometry acceptance.'))
if __name__=='__main__':
 acquire()
 try:main(int(sys.argv[sys.argv.index('--')+1]))
 finally:release()
