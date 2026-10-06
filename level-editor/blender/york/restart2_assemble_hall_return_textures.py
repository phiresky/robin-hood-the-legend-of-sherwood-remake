"""Reuse verified hall material data on each exact approved corrected state."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import sys

ROOT = Path(__file__).resolve().parents[3]
BASE = ROOT / 'level-editor/work/york-refinement'
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('state', choices=['initial-initial', 'initial-applied', 'applied-initial'])
args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
sha = lambda p: hashlib.sha256(Path(p).read_bytes()).hexdigest()
archive = BASE / 'restart2/hall-return-four-states-v1/approval-batch-v8'
receipt = json.loads((archive / 'archive.json').read_text())
member = next(r for r in receipt['members'] if r['state'] == args.state)
model = Path(member['model'])
if sha(model) != member['model_sha256']:
    raise ValueError('Approved state changed')
if sha(archive / 'user-approval.json') != receipt['receipt_sha256']:
    raise ValueError('User approval changed')
output = BASE / 'restart2/hall-textures-v2' / args.state / 'assembled-v1'
if output.exists():
    raise FileExistsError(output)
if shutil.disk_usage(ROOT).free < 25 * 1024**3:
    raise RuntimeError('Disk below25GiB')
common = BASE / 'restart2/hall-textures-v2/applied-applied/bake-v1'
cover = BASE / 'restart2/hall-textures-v1' / args.state / 'bake-cover-v1'
for donor in [common, cover]:
    validation = json.loads((donor / 'validation.json').read_text())
    if sha(donor / 'model.blend') != validation['baked_model_sha256']:
        raise ValueError('Donor changed after validation')
if sha(common / 'model.blend') != '64de1674aa4e4e1696c0b60dbc360dc036275d8f1c9f3bc64df69b8f61f99f6a':
    raise ValueError('Unexpected common donor')
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
from render_slots import acquire
acquire()
sys.path.insert(0, str(ROOT / 'level-editor/blender/nottingham'))
from freeze_tooling import select_tooling
select_tooling(json.loads((BASE / 'tooling/current.json').read_text())['directory'])
import bpy
from refinement_workspace import _geometry
from render_multiview_asset import render
bpy.ops.wm.open_mainfile(filepath=str(model))
scene = bpy.data.scenes['york Refinement']
bpy.context.window.scene = scene
bpy.context.view_layer.update()
scene.render.threads_mode = 'FIXED'
scene.render.threads = 2
asset = 'york-castle-great-hall'
before = {o.name: _geometry(o) for o in scene.objects}
outside = {o.name: _geometry(o, protect_appearance=True) for o in scene.objects if o.get('asset_group') != asset}
cover_nodes = {'building-799', 'scenery-york-great-hall-upper-front-wall'}
records = []
for donor_path, is_cover in [(common, False), (cover, True)]:
    targets = {o.name: o for o in scene.objects if o.type == 'MESH' and not o.hide_render
               and o.get('asset_group') == asset
               and (o.get('source_node') in cover_nodes) == is_cover}
    expected = (1 if args.state == 'initial-applied' else 2) if is_cover else 13
    if len(targets) != expected:
        raise ValueError(f'Unexpected receiver count: {len(targets)} != {expected}')
    with bpy.data.libraries.load(str(donor_path / 'model.blend'), link=False) as (available, imported):
        if not set(targets).issubset(available.objects):
            raise ValueError('Missing material donor')
        imported.objects = list(targets)
    for name, donor in zip(targets, imported.objects):
        target = targets[name]
        if donor.get('asset_group') != asset or any(donor.get(k) != target.get(k)
            for k in ['source_node', 'projection_component']):
            raise ValueError('Foreign material donor')
        # Retain reopened state transforms and visibility; transfer local data only.
        target.data = donor.data.copy()
        records.append({'receiver': name, 'donor': str(donor_path / 'model.blend'),
                        'donor_sha256': sha(donor_path / 'model.blend')})
    for donor in imported.objects:
        bpy.data.objects.remove(donor, do_unlink=True)
    bpy.context.view_layer.update()
if before != {o.name: _geometry(o) for o in scene.objects}:
    raise ValueError('Material reuse changed approved geometry')
if outside != {o.name: _geometry(o, protect_appearance=True) for o in scene.objects if o.name in outside}:
    raise ValueError('Material reuse changed outside appearance')
output.mkdir(parents=True)
bpy.context.preferences.filepaths.save_version = 0
bpy.ops.wm.save_as_mainfile(filepath=str(output / 'model.blend'), compress=True)
(output / 'assembly.json').write_text(json.dumps({
    'state': args.state, 'geometry_preserved': True, 'outside_appearance_preserved': len(outside),
    'approved_model_sha256': member['model_sha256'], 'user_receipt_sha256': receipt['receipt_sha256'],
    'material_transfers': records, 'baked_model_sha256': sha(output / 'model.blend'),
    'status': 'Private material candidate; native known-source and eight-view review required'
}, indent=2) + '\n')
views = BASE / 'restart2/hall-textures-v1' / args.state / 'experiment/views.json'
render(views, output / 'actual', width=384)
print(output)
