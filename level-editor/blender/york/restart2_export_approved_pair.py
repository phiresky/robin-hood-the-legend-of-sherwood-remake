"""Export the exact approved market pair into a private, pivot-preserving library."""
import hashlib,json,shutil,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/york-refinement';PAIR=BASE/'restart2/pair-textures-v1/assembled-v3-bounded';OUT=PAIR/'export-v1';ARC=PAIR/'approval-batch-v5'
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
if OUT.exists():raise FileExistsError(OUT)
if shutil.disk_usage(ROOT).free<25*1024**3:raise RuntimeError('Disk below25GiB')
archive=json.loads((ARC/'archive.json').read_text());source=ARC/'model.blend'
assert sha(source)==archive['model_sha256']=='ca627395685083078efe6edb51d38c44bfca84cc8ec7ed89bbfe4ffe3077d9d7'
assert sha(ARC/'user-approval.json')==archive['receipt_sha256']
ids=[m['asset_id'] for m in archive['members']]
if len(ids)!=2 or any(m['scope']!='texture' for m in archive['members']):raise ValueError('Unexpected approval scope')
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
import bpy
from export_editor import export_asset_library
from refinement_workspace import _geometry
bpy.ops.wm.open_mainfile(filepath=str(source));bpy.context.window.scene=bpy.data.scenes['york Refinement'];bpy.context.view_layer.update()
working=bpy.data.collections['york Working'];members=[o for o in working.all_objects if o.type=='MESH' and not o.hide_render and o.get('asset_group') in ids]
expected={'building-309','building-310','building-311','building-312','building-313'}
assert {o.get('source_node') for o in members}==expected
before={o.name:_geometry(o,protect_appearance=True) for o in members}
live={a:ROOT/'level-editor/library/3d-assets/york'/a/'asset.json' for a in ids};pins={str(p):sha(p) for p in live.values()};pivots={a:json.loads(p.read_text())['source_origin_scene'] for a,p in live.items()}
report=export_asset_library('york',OUT/'3d-assets',BASE/'baseline/york.rhp.json',standalone_pivots=pivots,asset_ids=ids,catalog=json.loads((ROOT/'level-editor/refinement/catalogs/york.json').read_text()))
assert before=={o.name:_geometry(o,protect_appearance=True) for o in members}
assert pins=={str(p):sha(p) for p in live.values()}
assert sha(source)==archive['model_sha256']
(OUT/'export-report.json').write_text(json.dumps({'status':'Private export complete; round-trip and editor validation pending','approved_model_sha256':sha(source),'approval_receipt_sha256':sha(ARC/'user-approval.json'),'assets':ids,'preserved_source_pivots':pivots,'source_geometry_uv_materials_unchanged':True,'source_components':sorted(expected),'live_descriptor_pins':pins,'report':report,'files':{str(p.relative_to(OUT)):sha(p) for p in sorted(OUT.rglob('*')) if p.is_file()},'live_integrated':False},indent=2)+'\n')
