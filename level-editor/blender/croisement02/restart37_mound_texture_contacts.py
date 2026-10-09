"""Review filled mound appearance against every retained ground/bank/wall contact."""
from pathlib import Path
import sys,json,hashlib,math
import shutil
import bpy
from mathutils import Vector,Matrix
P=Path(__file__).resolve().parent;sys.path.insert(0,str(P))
from restart6_source_gap_audit import OUT,RAY,SIN,COS

from restart4_stump_final_contact import frame,sheet
from render_slots import acquire,release
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def load_receivers(model,validation):
 # Load only the three required receiver families; never duplicate the full static scene.
 assert shutil.disk_usage(model).free>25*1024**3,'Disk reserve reached before scene load'
 bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene
 retained={}
 for obj in list(scene.objects):
  if obj.type=='MESH':
   retained[obj.name]=obj
   for col in list(obj.users_collection):col.objects.unlink(obj)
 objects=[];pins=[]
 for pin in validation['substitutions']:
  if pin['group']not in ['GROUND','croisement02-north-woodland-bank']:continue
  path=Path(pin['model']);assert sha(path)==pin['sha256']
  with bpy.data.libraries.load(str(path),link=False)as(src,dst):dst.scenes=src.scenes
  selected=[]
  for imported in dst.scenes:
   bpy.context.window.scene=imported;bpy.context.view_layer.update()
   for obj in imported.objects:
    if obj.type=='MESH'and((pin['group']=='GROUND'and obj.name.startswith('Croisement02 Terrain'))or obj.get('asset_group')==pin['group']):selected.append((obj,obj.matrix_world.copy()))
  bpy.context.window.scene=scene;assert selected
  for obj,matrix in selected:scene.collection.objects.link(obj);obj.parent=None;obj.matrix_world=matrix;obj.hide_render=False;objects.append(obj)
  pins.append(dict(group=pin['group'],model=str(path),sha256=pin['sha256'],objects=[dict(name=o.name,matrix_world=[list(v)for v in m])for o,m in selected]))
 authority=OUT/'restart2-textures/batch10-linked-static-v1/source-pins.json';rows=[r for r in json.loads(authority.read_text())['receivers'].values()if r['asset_group']=='croisement02-southeast-stone-wall-and-gate'and not r['hide_render']];assert rows
 for source in sorted({r['model']for r in rows}):
  chosen=[r for r in rows if r['model']==source];assert sha(Path(source))==chosen[0]['model_sha256']
  with bpy.data.libraries.load(source,link=False)as(src,dst):dst.objects=[r['object_name']for r in chosen]
  for row,obj in zip(chosen,dst.objects):assert obj is not None;scene.collection.objects.link(obj);obj.parent=None;obj.matrix_world=Matrix(row['matrix_world']);obj.hide_render=False;objects.append(obj)
  pins.append(dict(group='croisement02-southeast-stone-wall-and-gate',model=source,sha256=chosen[0]['model_sha256'],objects=chosen,authority_sha256=sha(authority)))
 bpy.context.view_layer.update();base=OUT/'restart2-textures/batch10-linked-static-v1/scene.blend';assert sha(base)==validation['static_base_sha256'];return scene,objects,pins,base,retained

def main():
 worker=OUT/'restart25-approved-state-materialization-v1/mound-filled-all-sites-v1';r=json.loads((OUT/'restart15-hiding-mounds/all-placements-v1/validation.json').read_text());model=worker/'worker.blend';proof=json.loads((worker/'preservation.json').read_text());assert sha(model)==proof['model_sha256'];out=worker/'contacts-v2';out.mkdir(exist_ok=False);scene,static,pins,base,mapped=load_receivers(model,r);ground=[o for o in static if o.name.startswith('Croisement02 Terrain')or o.get('asset_group')=='croisement02-north-woodland-bank'];wall=[o for o in static if o.get('asset_group')=='croisement02-southeast-stone-wall-and-gate'];names=[n for row in r['records']for n in row['objects']]
 for name in names:scene.collection.objects.link(mapped[name])
 bpy.context.view_layer.update();scene.cycles.samples=16;scene.cycles.transparent_max_bounces=512;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';scene.view_settings.look='None';images=[]
 for row in r['records']:
  own=[mapped[n]for n in row['objects']];receivers=ground+wall
  for obj in scene.objects:
   if obj.type=='MESH':obj.hide_render=obj not in own and obj not in receivers
  paths=[]
  for view,direction in [('native',RAY),('side',Vector((COS,0,SIN))),('low-side',Vector((math.cos(math.radians(12)),0,math.sin(math.radians(12)))))]:
   camera=frame(scene,own,direction,480,1.5);file=out/f'{row["tag"]}-{view}.png';scene.render.filepath=str(file);assert shutil.disk_usage(out).free>25*1024**3,'Disk reserve reached before render';bpy.ops.render.render(write_still=True);paths.append(file);images.append(dict(tag=row['tag'],view=view,path=file.name,sha256=sha(file),camera_matrix=[list(v)for v in camera.matrix_world],receivers=[o.name for o in receivers]))
  sheet(paths,out/f'{row["tag"]}-contact-three.png')
 sheet([out/r['path']for r in images],out/'all-contact-views.png');(out/'report.json').write_text(json.dumps(dict(model_sha256=sha(model),static_base_sha256=sha(base),substitutions=pins,images=images,scope='Ground/bank and relevant wall only. Canopy omitted to expose contact. Native camera first, two oblique support views. No full-scene occlusion claim.'),indent=2)+'\n')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
