"""Project the revised hall-wall source authority onto the bounded return control."""
import argparse,hashlib,json,shutil,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/york-refinement';parser=argparse.ArgumentParser();parser.add_argument('--version',default='hall-return-control-v4-source');parser.add_argument('--source',type=Path,default=BASE/'restart2/hall-return-control-v3-context/model.blend');parser.add_argument('--receiver',default='building-791');args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else []);OUT=BASE/'restart2'/args.version
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
source=args.source;bpy.ops.wm.open_mainfile(filepath=str(source));scene=bpy.data.scenes['york Refinement'];bpy.context.window.scene=scene;bpy.context.view_layer.update()
nodes={args.receiver};geometry={o.name:_geometry(o) for o in scene.objects};outside={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o.get('source_node') not in nodes};OUT.mkdir()
bake('york',BASE/'restart2/hall-cover-source-combinations-v1/patch001-applied_patch002-applied.png',OUT/'projection.json',receiver_nodes=[args.receiver],receiver_asset_id='york-castle-great-hall',projection_label='hall-semantic-applied-applied',source_mask_manifest=BASE/'restart2/hall-source-authority-v2/source-masks.json',texels_per_unit=2,preserve_authored=False)
assert geometry=={o.name:_geometry(o) for o in scene.objects}
assert outside=={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o.get('source_node') not in nodes}
bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'model.blend'),compress=True)
(OUT/'source-refresh.json').write_text(json.dumps({'scope':'Single named hall receiver native source appearance renewed under reviewed component authority; private geometry control','refreshed_nodes':sorted(nodes),'all_geometry_unchanged':True,'all_other_appearance_unchanged':True,'source_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'model_sha256':hashlib.sha256((OUT/'model.blend').read_bytes()).hexdigest(),'authority_sha256':hashlib.sha256((BASE/'restart2/hall-source-authority-v2/source-masks.json').read_bytes()).hexdigest()},indent=2)+'\n')
from render_multiview_asset import render
render(BASE/'restart2/hall-textures-v1/applied-applied/experiment/views.json',OUT/'actual',width=384)
