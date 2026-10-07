"""Inspect every propagated mound within the frozen physical neighborhood."""
from pathlib import Path
import sys,json,hashlib,math
import bpy
from mathutils import Vector
P=Path(__file__).resolve().parent;sys.path.insert(0,str(P))
from restart6_source_gap_audit import OUT,RAY,SIN,COS
from leaf_state_scene_context import load_scene
from restart4_stump_final_contact import frame,sheet
from render_slots import acquire,release
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 worker=OUT/'restart15-hiding-mounds/all-placements-v1';r=json.loads((worker/'validation.json').read_text());model=worker/'model.blend';assert sha(model)==r['model_sha256'];out=worker/'neighborhoods-v1';out.mkdir(exist_ok=False);scene,static,pins,base=load_scene();ground=[o for o in static if o.name.startswith('Croisement02 Terrain')or o.get('asset_group')=='croisement02-north-woodland-bank'];wall=[o for o in static if o.get('asset_group')=='croisement02-southeast-stone-wall-and-gate'];names=[n for row in r['records']for n in row['objects']]
 with bpy.data.libraries.load(str(model),link=False)as(src,dst):dst.objects=list(names)
 mapped={}
 for name,obj in zip(names,dst.objects):scene.collection.objects.link(obj);mapped[name]=obj
 bpy.context.view_layer.update();scene.cycles.samples=16;scene.cycles.transparent_max_bounces=512;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';scene.view_settings.look='None';images=[]
 for row in r['records']:
  own=[mapped[n]for n in row['objects']];receivers=static
  for obj in scene.objects:
   if obj.type=='MESH':obj.hide_render=obj not in own and obj not in receivers
  paths=[]
  for view,direction in [('native',RAY),('oblique',Vector((COS,0,SIN)))]:
   camera=frame(scene,own,direction,480,2.0);file=out/f'{row["tag"]}-{view}.png';scene.render.filepath=str(file);bpy.ops.render.render(write_still=True);paths.append(file);images.append(dict(tag=row['tag'],view=view,path=file.name,sha256=sha(file),camera_matrix=[list(v)for v in camera.matrix_world],receivers=[o.name for o in receivers]))
  sheet(paths,out/f'{row["tag"]}-contact-three.png')
 sheet([out/r['path']for r in images],out/'all-neighborhood-views.png');(out/'report.json').write_text(json.dumps(dict(model_sha256=sha(model),static_base_sha256=sha(base),substitutions=pins,images=images,scope='All frozen static neighbors included, with512 transparent bounces; native camera first and an oblique. Does not establish mission source ordering or moving actor parity.'),indent=2)+'\n')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
