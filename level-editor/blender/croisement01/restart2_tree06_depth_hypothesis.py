"""Private native-ray placement hypothesis; preserves geometry and painted UVs."""
import json,math,shutil,sys
from pathlib import Path
import bpy
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
from review_evidence import sha
R=ROOT/'level-editor/work/croisement01-refinement/restart2';src=R/'tree06-v7/assets/croisement01-tree-06';out=R/'tree06-v8/assets/croisement01-tree-06';assert not out.exists();acquire();bpy.ops.wm.open_mainfile(filepath=str(src/'model.blend'));bpy.context.preferences.filepaths.save_version=0
shift=Vector((0,20/math.tan(math.radians(35)),-20));changed=[];max_projection=0
for obj in bpy.context.scene.objects:
 if obj.type!='MESH' or obj.get('asset_group')!='croisement01-tree-06':continue
 local=obj.matrix_world.inverted().to_3x3()@shift
 for v in obj.data.vertices:
  old=obj.matrix_world@v.co;v.co+=local;new=obj.matrix_world@v.co
  max_projection=max(max_projection,abs((new.y-old.y)*math.sin(math.radians(35))+(new.z-old.z)*math.cos(math.radians(35))))
 changed.append(obj.get('source_node'))
assert len(changed)==2 and max_projection<.001
(out/'modified').mkdir(parents=True);(out/'inspection').mkdir();shutil.copy2(src/'workspace.json',out/'workspace.json');packet=json.loads((src/'modified/views.json').read_text())
for view in packet['views']:
 for i in range(3):view['camera_location'][i]+=shift[i];view['camera_matrix_world'][i][3]+=shift[i]
(out/'modified/views.json').write_text(json.dumps(packet,indent=2)+'\n');shutil.copy2(R/'tree06-v7/wood-domain.png',R/'tree06-v8/wood-domain.png');bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'))
(R/'tree06-v8/depth-hypothesis.json').write_text(json.dumps(dict(status='Private joint hypothesis, not approved',source_sha256=sha(src/'model.blend'),model_sha256=sha(out/'model.blend'),world_translation=list(shift),native_projection_max_error=max_projection,changed_nodes=changed,geometry_uv_topology_unchanged=True,rationale='Lower inferred root elevation by 20 world units along original camera rays; broad soil slope must support it. Coarse archived bank elevation is not a native depth observation.'),indent=2)+'\n');print(out)
