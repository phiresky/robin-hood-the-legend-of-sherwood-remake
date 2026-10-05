"""Lower the small basal gap along the native projection ray, preserving the image contour."""
import sys,json
from pathlib import Path
import bpy,bmesh
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from refinement_workspace import _geometry
from render_slots import acquire,release
from tree_geometry import RAY
from rebuild_tree32_roots import check

def main():
 source=OUT/'restart2-wood/tree46-branch-fitted-v2';out=OUT/'restart2-wood/tree46-branch-fitted-v3';out.mkdir(exist_ok=False);digest=sha(source/'model.blend')
 bpy.ops.wm.open_mainfile(filepath=str(source/'model.blend'));bpy.context.view_layer.update();bpy.context.preferences.filepaths.save_version=0
 objects=list(bpy.data.collections['Croisement02 Working'].all_objects);wood=[o for o in objects if o.type=='MESH' and o.get('asset_group')=='croisement02-tree-46' and o.get('projection_component')!='crown'];protected={o.name:_geometry(o,protect_appearance=True) for o in objects if o.type=='MESH' and o not in wood};minimum=min((o.matrix_world@v.co).z for o in wood for v in o.data.vertices);drop=minimum+.2
 if not 0<drop<2:raise ValueError('Unexpected basal gap')
 maximum=0
 for obj in wood:
  inverse=obj.matrix_world.inverted()
  for v in obj.data.vertices:
   p=obj.matrix_world@v.co;t=max(0.,min(1.,(30.-p.z)/28.));weight=t*t*(3-2*t);shift=-RAY*(drop*weight/RAY.z);maximum=max(maximum,shift.length);v.co=inverse@(p+shift)
  obj.data.update();check(obj.data)
 if protected!={o.name:_geometry(o,protect_appearance=True) for o in objects if o.type=='MESH' and o not in wood}:raise ValueError('Other appearance changed')
 bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'));report=json.loads((source/'evidence.json').read_text());report.update(model_sha256=sha(out/'model.blend'),basal_projection_ray_repair=dict(input_model=str(source/'model.blend'),input_sha256=digest,old_minimum_z=minimum,new_minimum_z=min((o.matrix_world@v.co).z for o in wood for v in o.data.vertices),maximum_displacement=maximum,scope='Only z<30, full shift z<=2; native projected vertex positions unchanged',status='Inferred flat-ground contact; independent review required'));write_json(out/'evidence.json',report)
 if sha(source/'model.blend')!=digest:raise ValueError('Source changed')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
