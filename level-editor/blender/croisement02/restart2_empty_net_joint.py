"""Check the empty net against evaluated current tree supports without moving neighbors."""
import sys,json
from pathlib import Path
import bpy,bmesh,numpy as np
from mathutils import Vector
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT,tree_workspace
from evidence_io import sha,write_json
from render_slots import acquire,release
from sign_context_import import append_verified
from log_trap_state_candidate import point
from tree_geometry import RAY
BASE=OUT/'restart2-state/net-empty01-v7';DEST=BASE/'joint'
def main():
 if DEST.exists():raise FileExistsError(DEST)
 acquire()
 try:
  DEST.mkdir();bindings=[]
  for index in [43,45,46]:
   worker=tree_workspace(index);workspace=json.loads((worker/'workspace.json').read_text());bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'));bpy.context.window.scene=bpy.data.scenes[workspace['scene_name']];bpy.context.view_layer.update();names=json.loads((worker/'modified/views.json').read_text())['object_names'];expected={name:{'matrix_world':[list(r)for r in bpy.data.objects[name].matrix_world]}for name in names};bindings.append({'tree':index,'worker':str(worker),'model_sha256':sha(worker/'model.blend'),'names':names,'expected':expected})
  digest=sha(BASE/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(BASE/'model.blend'));scene=bpy.context.scene;imports=[];wood=[];foliage=[]
  for row in bindings:
   objects,receipts=append_verified(scene,Path(row['worker'])/'model.blend',row['names'],row['expected']);imports.extend(receipts)
   for obj in objects:
    (wood if 'wood 'in obj.name else foliage).append(obj)
  intersections=[]
  for name in ['Empty bag','Wooden piece']:
   body=bpy.data.objects[name]
   for receiver in wood:
    test=body.copy();test.data=body.data.copy();scene.collection.objects.link(test);bpy.context.view_layer.objects.active=test;mod=test.modifiers.new('Independent support intersection','BOOLEAN');mod.operation='INTERSECT';mod.solver='EXACT';mod.object=receiver;bpy.ops.object.modifier_apply(modifier=mod.name);bm=bmesh.new();bm.from_mesh(test.data);volume=abs(bm.calc_volume(signed=True));bm.free();intersections.append({'body':name,'receiver':receiver.name,'volume':volume});bpy.data.objects.remove(test,do_unlink=True)
  center=point(1330,1010,135);scene.camera.data.ortho_scale=245;scene.render.resolution_x=scene.render.resolution_y=640;scene.cycles.samples=16;records=[]
  for name,direction,only_wood in [('native-context',RAY,False),('native-wood',RAY,True),('reverse-wood',Vector((0,1,.8)).normalized(),True),('oblique-wood',Vector((1,1,.8)).normalized(),True)]:
   for obj in foliage:obj.hide_render=only_wood
   scene.camera.location=center+direction*3000;scene.camera.rotation_euler=(center-scene.camera.location).to_track_quat('-Z','Y').to_euler();file=DEST/(name+'.png');scene.render.filepath=str(file);bpy.ops.render.render(write_still=True);records.append({'name':name,'image':file.name,'direction':list(direction),'sha256':sha(file)})
  write_json(DEST/'report.json',{'model_sha256':digest,'bindings':bindings,'evaluated_imports':imports,'body_tree_intersections':intersections,'renders':records,'limits':['Current tree geometry only; pending new tree texture fills not selected.','Original opaque native order is a separate presentation contract.','Single empty endpoint; no temporal rigid identity claim.']})
 finally:release()
if __name__=='__main__':main()
