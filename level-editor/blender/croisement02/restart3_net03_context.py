"""Verify exact nearby tree transforms and converged transparent net context."""
import json,math,sys
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from tree_geometry import RAY,SIN,COS
from restore_ground75_source import geometry
from review_bank_candidate import camera
from render_slots import acquire,release


def main():
 base=OUT/'restart3-net03/endpoints-v4';survey=OUT/'restart3-net03/support-survey-v1/manifest.json';neighbors=json.loads(survey.read_text())['records'];frozen=[];nety=-629/SIN-50*COS
 acquire()
 try:
  for row in neighbors:
   model=Path(row['worker']);assert sha(model)==row['model_sha256'];bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update();objects=[o for o in bpy.context.scene.objects if o.type=='MESH'and o.get('asset_group')==f"croisement02-tree-{row['tree']:02}"];frozen.append(dict(tree=row['tree'],model=model,sha256=row['model_sha256'],signatures={o.name:geometry(o)for o in objects}))
  for suffix in ['e','i']:
   folder=base/suffix;dest=folder/'context-budget-v1'
   if dest.exists():raise FileExistsError(dest)
   dest.mkdir();report=json.loads((folder/'manifest.json').read_text());assert sha(folder/'model.blend')==report['model_sha256'];bpy.ops.wm.open_mainfile(filepath=str(folder/'model.blend'));scene=bpy.context.scene;imported=[]
   for row in frozen:
    with bpy.data.libraries.load(str(row['model']),link=False)as(src,dst):dst.objects=list(row['signatures'])
    for o in dst.objects:scene.collection.objects.link(o)
    for o in dst.objects:
     parent=o.parent
     while parent is not None:
      if parent.name not in scene.objects:scene.collection.objects.link(parent)
      parent=parent.parent
    bpy.context.view_layer.update()
    for o in dst.objects:
     if geometry(o)!=row['signatures'][o.name]:raise ValueError('Imported source transform/geometry changed: '+o.name)
     o.hide_render=False
    imported.extend(dst.objects)
   target=Vector((1665,nety,155));images=[]
   for budget in [256,512]:
    camera(scene,target,Vector(RAY),640,640,370);scene.cycles.transparent_max_bounces=budget;scene.cycles.seed=0;scene.render.filepath=str(dest/f'native-{budget}.png');bpy.ops.render.render(write_still=True);images.append(np.array(Image.open(scene.render.filepath).convert('RGBA')))
   changed=np.any(images[0]!=images[1],axis=2);max_delta=int(np.max(np.abs(images[0].astype(int)-images[1].astype(int))))
   if changed.any():
    camera(scene,target,Vector(RAY),640,640,370);scene.cycles.transparent_max_bounces=1024;scene.cycles.seed=0;scene.render.filepath=str(dest/'native-1024.png');bpy.ops.render.render(write_still=True);last=np.array(Image.open(scene.render.filepath).convert('RGBA'));final_changed=int(np.any(images[1]!=last,axis=2).sum());budget=1024
   else:final_changed=0;budget=512
   if final_changed:raise ValueError('Transparent context did not converge at512/1024')
   for i,angle in enumerate([math.pi/4,-math.pi/3],1):
    direction=Vector((math.sin(angle)*COS,-math.cos(angle)*COS,SIN));camera(scene,target,direction,640,640,370);scene.cycles.transparent_max_bounces=budget;scene.cycles.seed=0;scene.render.filepath=str(dest/f'contact-{i}.png');bpy.ops.render.render(write_still=True)
   for o in imported:
    if o.get('projection_component')=='crown':o.hide_render=True
   camera(scene,Vector((1655,nety,145)),Vector((COS*.7071,-COS*.7071,SIN)),640,640,300);scene.cycles.transparent_max_bounces=budget;scene.render.filepath=str(dest/'wood-only.png');bpy.ops.render.render(write_still=True)
   write_json(dest/'validation.json',dict(status='PASS exact neighbor transforms and converged native transparency',model_sha256=report['model_sha256'],source_camera_first=True,neighbors=[dict(tree=r['tree'],model=str(r['model']),model_sha256=r['sha256'],geometry_uv_world_signatures=r['signatures'])for r in frozen],all_imported_geometry_uv_world_exact=True,transparent_budgets=[256,512],changed_pixels_256_to_512=int(changed.sum()),maximum_rgba_delta_256_to_512=max_delta,final_changed_pixels=final_changed,final_budget=budget,standalone_opaque_materials=True,images={p.name:sha(p)for p in dest.glob('*.png')}))
 finally:release()
if __name__=='__main__':main()
