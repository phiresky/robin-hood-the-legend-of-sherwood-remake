"""Extend the counterweight underside to its observed native contour."""
import sys,json
from pathlib import Path
import bpy,bmesh
from mathutils import Vector
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import SIN,COS
BASE=OUT/'restart2-state/net-empty01-v9';DEST=OUT/'restart2-state/net-empty01-v10'
def main():
 if DEST.exists():raise FileExistsError(DEST)
 acquire()
 try:
  DEST.mkdir();prior=json.loads((BASE/'report.json').read_text());assert sha(BASE/'model.blend')==prior['model_sha256'];bpy.ops.wm.open_mainfile(filepath=str(BASE/'model.blend'));scene=bpy.context.scene;wood=bpy.data.objects['Wooden piece'];changes=[]
  for v in wood.data.vertices:
   world=wood.matrix_world@v.co;sy=-world.y*SIN-world.z*COS-1006;delta=max(0,sy-27)*.16;world.z-=delta/COS;v.co=wood.matrix_world.inverted()@world;changes.append(delta)
  wood.data.update();uv=wood.data.uv_layers['Native target projection']
  for loop in wood.data.loops:
   p=wood.matrix_world@wood.data.vertices[loop.vertex_index].co;uv.data[loop.index].uv=((p.x-1296)/52,1-(-p.y*SIN-p.z*COS-1006)/62)
  # Preserve the bag exterior and remove only newly occupied counterweight volume.
  bag=bpy.data.objects['Empty bag'];bpy.context.view_layer.objects.active=bag;mod=bag.modifiers.new('Updated counterweight contact','BOOLEAN');mod.operation='DIFFERENCE';mod.solver='EXACT';mod.object=wood;bpy.ops.object.modifier_apply(modifier=mod.name)
  import shutil
  for path in BASE.glob('*observed.png'):shutil.copyfile(path,DEST/path.name)
  bpy.ops.wm.save_as_mainfile(filepath=str(DEST/'model.blend'));write_json(DEST/'report.json',{**prior,'status':'Private source-supported counterweight underside correction; audits pending','model_sha256':sha(DEST/'model.blend'),'parent_model_sha256':prior['model_sha256'],'counterweight_source_y_delta_max':max(changes),'renders':[]})
 finally:release()
if __name__=='__main__':main()
