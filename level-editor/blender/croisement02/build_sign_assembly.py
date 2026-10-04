"""Create five mission-only instances with native placement and shared sign geometry."""
import json,sys,math
from pathlib import Path
import bpy
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release

def main():
 base=OUT/'state-sign-candidate';candidate=base/'candidate-v2';model=candidate/'model.blend';validation=json.loads((candidate/'validation.json').read_text());assert sha(model)==validation['model_sha256'];placement=json.loads((base/'placement-audit.json').read_text());assert all(abs(r['physical_clearance'])<.01 for r in placement['records']);dst=base/'five-instances-v1';dst.mkdir(exist_ok=False)
 bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;scene.name='Croisement02 S03 FoB MP signs';root=scene.objects['Rotating sign pivot'];parts=[scene.objects['Panneau '+s]for s in ['board','post']];rows=[]
 for row in placement['records']:
  index=row['target_index'];new_root=root.copy();new_root.name=f'Mission S03 FoB MP sign {index}';scene.collection.objects.link(new_root);new_root.location=Vector(row['world_anchor']);new_root['native_target_index']=index;new_root['mission_visibility']=row['mission'];new_root['native_placement_json']=json.dumps(row['native_target'],sort_keys=True);new_root['native_implicit_z']=row['native_implicit_z'];new_root['source_node']=f'mission-panneau-{index}';children=[]
  for part in parts:
   copy=part.copy();copy.name=f'{new_root.name} {part["part_name"]}';scene.collection.objects.link(copy);copy.parent=new_root;copy['source_node']=new_root['source_node'];children.append(copy.name)
  rows.append(dict(target_index=index,mission=row['mission'],source_node=new_root['source_node'],root=new_root.name,parts=children,world_anchor=row['world_anchor'],native_target=row['native_target'],reusable_asset_id='croisement02-mission-rotating-sign',native_pose_action_aliases=[0,210,211]))
 for obj in [*parts,root]:bpy.data.objects.remove(obj,do_unlink=True)
 for obj in list(scene.objects):
  if obj.type not in {'MESH','EMPTY'}:bpy.data.objects.remove(obj,do_unlink=True)
 scene.frame_set(1);bpy.ops.wm.save_as_mainfile(filepath=str(dst/'model.blend'));digest=sha(dst/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(dst/'model.blend'));scene=bpy.context.scene;scene.frame_set(1);assert len([o for o in scene.objects if o.type=='MESH'])==10
 for row in rows:
  pivot=scene.objects[row['root']];assert (pivot.location-Vector(row['world_anchor'])).length<1e-4
  for name in row['parts']:assert scene.objects[name].parent==pivot
 write_json(dst/'assembly.json',dict(status='Private mission geometry assembly; independent review and appearance integration pending',model_sha256=digest,reusable_model_sha256=validation['model_sha256'],placement_audit_sha256=sha(base/'placement-audit.json'),candidate_validation_sha256=sha(candidate/'validation.json'),instances=rows,mesh_count=10,shared_mesh_datablocks=2,mission_visibility='Only S03_FoB_MP; no permanent all-mission signs',timing=dict(ticks_per_second=25,cycle_ticks=64,poses=32,pose_ticks=2),remaining=['Independent saved-model geometry review','Gray hidden texel completion','Native shadow and animated canopy compositing validation','Catalog/export/editor integration']))
 print(dst)

if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
