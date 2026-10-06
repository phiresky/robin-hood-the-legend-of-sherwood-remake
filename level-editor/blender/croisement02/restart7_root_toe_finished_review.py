"""Native-first fixed-camera full and contact comparison, using map review sunlight."""
import sys,json
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector,Matrix
from PIL import Image
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
from restart6_source_gap_audit import ROOT,OUT,RAY,SIN
from restart6_tree38_contour import covered
from restart4_stump_final_contact import frame,sheet
from render_multiview_asset import render
from review_sunlight import render_solids
from evidence_io import sha,write_json
from render_slots import acquire,release

def main(number):
 model=ROOT/f'tree{number}-toe-finished-v12'/'model.blend';out=ROOT/f'tree{number}-toe-review-v12';out.mkdir(exist_ok=False);prior=json.load(open(ROOT/f'baseline-audit-{number}-v3/report.json'));source=Path(prior['source']);asset=f'croisement02-tree-{number}';views=next(p/'views.json'for p in source.parents if(p/'views.json').exists());base=json.load(open(views));limit=42 if number==19 else 70;fullcams=[];localcams=[];results=[];m=next(x for x in json.load(open(OUT/'review-mask-inventory.json'))['masks']if x['index']==number);yy,xx=np.where(np.array(Image.open(m['png']))>0);targets=np.column_stack((xx+m['box_top_left'][0]+.5,yy+m['box_top_left'][1]+.5))
 for tag,path in [('before',source),('after',model)]:
  dest=out/tag;dest.mkdir();bpy.ops.wm.open_mainfile(filepath=str(path));bpy.context.view_layer.update();scene=bpy.context.scene;own=[o for o in scene.objects if o.type=='MESH'and o.get('asset_group')==asset];wood=[o for o in own if o.get('projection_component')!='crown'];results.append(covered(wood,targets));scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=512;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';scene.view_settings.look='None'
  for o in scene.objects:
   if o.type=='MESH':o.hide_render=o not in own
  manifest=json.loads(json.dumps(base));manifest['scene_name']=scene.name;manifest['object_names']=[o.name for o in own];manifest.pop('render_object_names',None)
  for i,v in enumerate(manifest['views']):
   if tag=='before':
    direction=RAY if i==0 else Matrix(v['camera_matrix_world']).to_quaternion()@Vector((0,0,1));cam=frame(scene,own,direction,384,1.25);fullcams.append((cam.matrix_world.copy(),cam.data.ortho_scale))
   v['camera_matrix_world']=[list(r)for r in fullcams[i][0]];v['ortho_scale']=fullcams[i][1];v['crop']={'width':384,'height':384}
  if tag=='before':
   points=[o.matrix_world@v.co for o in wood for v in o.data.vertices if(o.matrix_world@v.co).z<limit];mesh=bpy.data.meshes.new('Original lower framing');mesh.from_pydata(points,[],[]);proxy=bpy.data.objects.new(mesh.name,mesh);scene.collection.objects.link(proxy)
   for d in [RAY,Vector((.6124,-.6124,.5)),Vector((.6124,.6124,.5)),Vector((-.866,0,.5))]:
    cam=frame(scene,[proxy],d.normalized(),384,1.4);localcams.append((cam.matrix_world.copy(),cam.data.ortho_scale))
   write_json(dest/'reused-evidence.json',{'unchanged_original_review':str(ROOT/f'tree{number}-toe-review-v4/before'),'source_sha256':sha(source)});continue
  write_json(dest/'cameras.json',manifest);render(dest/'cameras.json',dest/'actual',modes=('textured','solid'),width=384)
  for mode in ['textured','solid']:sheet([dest/f'actual/view-{i}-{mode}.png'for i in range(8)],dest/f'{mode}8.png')
  for o in own:o.hide_render=o not in wood
  points=[o.matrix_world@v.co for o in wood for v in o.data.vertices if(o.matrix_world@v.co).z<limit];mesh=bpy.data.meshes.new('Lower framing');mesh.from_pydata(points,[],[]);proxy=bpy.data.objects.new(mesh.name,mesh);scene.collection.objects.link(proxy);proxy.hide_render=True;center=np.array(points).mean(0);bpy.ops.mesh.primitive_plane_add(size=180,location=(center[0],center[1],0));floor=bpy.context.object;floor.name='Z0 guide';mat=bpy.data.materials.new('Neutral guide');mat.diffuse_color=(.12,.14,.1,1);floor.data.materials.append(mat);dirs=[RAY,Vector((.6124,-.6124,.5)),Vector((.6124,.6124,.5)),Vector((-.866,0,.5))];cameras=[]
  for i,d in enumerate(dirs):
   cam=frame(scene,[proxy],d.normalized(),384,1.4)
   if tag=='before':localcams.append((cam.matrix_world.copy(),cam.data.ortho_scale))
   cam.matrix_world=localcams[i][0];cam.data.ortho_scale=localcams[i][1];cameras.append(cam);scene.render.filepath=str(dest/f'contact-{i}.png');bpy.ops.render.render(write_still=True)
  render_solids(scene,cameras,wood+[floor],dest,lighting=base['lighting']);sheet([dest/f'contact-{i}.png'for i in range(4)],dest/'contact4.png');sheet([dest/f'view-{i}-solid.png'for i in range(4)],dest/'solid-contact4.png');floor.hide_render=True
  info=json.load(open(ROOT/f'baseline-audit-{number}-v3/camera.json'));cam=bpy.data.objects.new('Native source close',bpy.data.cameras.new('Native source close'));scene.collection.objects.link(cam);cam.data.type='ORTHO';cam.data.ortho_scale=info['scale'];cam.data.clip_end=20000;cam.matrix_world=Matrix(info['camera']);scene.camera=cam;scene.render.resolution_x=scene.render.resolution_y=512;scene.render.resolution_percentage=100;scene.render.filepath=str(dest/'native-wood.png');bpy.ops.render.render(write_still=True);native=Image.open(ROOT/f'baseline-audit-{number}-v3/native-source.png').convert('RGBA');pic=Image.open(dest/'native-wood.png').convert('RGBA');comp=Image.new('RGBA',(1536,512),(35,35,35,255));comp.paste(native,(0,0));comp.paste(Image.alpha_composite(native,pic),(512,0));comp.paste(pic,(1024,0));comp.save(dest/'source-comparison.png')
 write_json(out/'evidence.json',dict(model=str(model),model_sha256=sha(model),source=str(source),source_sha256=sha(source),old_hits=int(results[0].sum()),new_hits=int(results[1].sum()),lost=(targets[results[0]&~results[1]]-.5).tolist(),gained=(targets[~results[0]&results[1]]-.5).tolist(),native_first=True,fixed_comparison_cameras=True,lighting=base['lighting']))
if __name__=='__main__':
 acquire()
 try:main(int(sys.argv[sys.argv.index('--')+1]))
 finally:release()
