"""Refresh newly exposed neighboring source receivers for a hall-only diagnostic."""
import hashlib,json,shutil,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/york-refinement';OUT=BASE/'restart2/hall-return-control-v3-context'
if OUT.exists():raise FileExistsError(OUT)
if shutil.disk_usage(ROOT).free<25*1024**3:raise RuntimeError('Disk below25GiB')
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
sys.path.insert(0,str(ROOT/'level-editor/blender/nottingham'))
from freeze_tooling import select_tooling
select_tooling(json.loads((BASE/'tooling/current.json').read_text())['directory'])
import bpy
from source_projection_bake import bake
from refinement_workspace import _geometry
source=BASE/'restart2/hall-return-control-v3/model.blend';bpy.ops.wm.open_mainfile(filepath=str(source));scene=bpy.data.scenes['york Refinement'];bpy.context.window.scene=scene;bpy.context.view_layer.update()
nodes={'building-770','building-805','building-806'};geometry={o.name:_geometry(o) for o in scene.objects};outside={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o.get('source_node') not in nodes};OUT.mkdir()
for group,receivers in [('york-castle-main-keep',['building-770']),('york-castle-northeast-square-watchtower',['building-805','building-806'])]:
 actual={o.get('asset_group') for o in scene.objects if o.get('source_node') in receivers}
 if len(actual)!=1:raise ValueError('Ambiguous context owner')
 group=next(iter(actual))
 bake('york',BASE/'restart2/hall-cover-source-combinations-v1/patch001-applied_patch002-applied.png',OUT/(receivers[0]+'.json'),receiver_nodes=receivers,receiver_asset_id=group,projection_label='private-newly-exposed-context',texels_per_unit=2,preserve_authored=False)
assert geometry=={o.name:_geometry(o) for o in scene.objects}
assert outside=={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o.get('source_node') not in nodes}
bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'model.blend'),compress=True)
(OUT/'context-refresh.json').write_text(json.dumps({'scope':'Private context-only native projection diagnostic; no neighbor geometry or source ownership approval','refreshed_nodes':sorted(nodes),'all_geometry_unchanged':True,'hall_and_other_appearance_unchanged':True,'source_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'model_sha256':hashlib.sha256((OUT/'model.blend').read_bytes()).hexdigest(),'reason':'These receivers were source-occluded by the removed791 return during earlier projection; refresh newly exposed native rays.'},indent=2)+'\n')
