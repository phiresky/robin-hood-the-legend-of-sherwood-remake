"""Keep common hall materials identical while retaining each approved roof state."""
import argparse
import hashlib
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
BASE = ROOT / 'level-editor/work/york-refinement'
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('cover_bake', type=Path)
parser.add_argument('common_bake', type=Path)
parser.add_argument('manifest', type=Path)
parser.add_argument('output', type=Path)
args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
if args.output.exists():
    raise FileExistsError(args.output)
sha = lambda p: hashlib.sha256(Path(p).read_bytes()).hexdigest()
for folder in (args.cover_bake, args.common_bake):
    validation = json.loads((folder / 'validation.json').read_text())
    if sha(folder / 'model.blend') != validation['baked_model_sha256']:
        raise ValueError('Bake changed after validation')
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
from render_slots import acquire
acquire()
sys.path.insert(0, str(ROOT / 'level-editor/blender/nottingham'))
from freeze_tooling import select_tooling
select_tooling(json.loads((BASE / 'tooling/current.json').read_text())['directory'])
import bpy
from refinement_workspace import _geometry
from render_multiview_asset import render

asset = 'york-castle-great-hall'
bpy.ops.wm.open_mainfile(filepath=str(args.cover_bake / 'model.blend'))
scene = bpy.data.scenes['york Refinement']
bpy.context.window.scene = scene
bpy.context.view_layer.update()
before = {o.name: _geometry(o) for o in scene.objects}
outside = {o.name: _geometry(o, protect_appearance=True) for o in scene.objects
           if o.get('asset_group') != asset}
targets = {o.name: o for o in scene.objects if o.type == 'MESH' and not o.hide_render
           and o.get('asset_group') == asset
           and o.get('source_node') not in ('building-799', 'scenery-york-great-hall-upper-front-wall')}
if len(targets) != 13:
    raise ValueError('Unexpected common component inventory')
with bpy.data.libraries.load(str(args.common_bake / 'model.blend'), link=False) as (available, imported):
    if not set(targets).issubset(available.objects):
        raise ValueError('Missing common donors')
    imported.objects = list(targets)
for name, donor in zip(targets, imported.objects):
    target = targets[name]
    if donor.get('asset_group') != asset or any(donor.get(k) != target.get(k)
        for k in ('source_node', 'projection_component')):
        raise ValueError('Foreign common donor')
    # Copy local data only: the reopened target retains its evaluated transform.
    target.data = donor.data.copy()
for donor in imported.objects:
    bpy.data.objects.remove(donor, do_unlink=True)
bpy.context.view_layer.update()
if before != {o.name: _geometry(o) for o in scene.objects}:
    raise ValueError('Common transfer changed approved geometry')
if outside != {o.name: _geometry(o, protect_appearance=True) for o in scene.objects if o.name in outside}:
    raise ValueError('Common transfer changed outside appearance')
args.output.mkdir()
bpy.context.preferences.filepaths.save_version = 0
bpy.ops.wm.save_as_mainfile(filepath=str(args.output / 'model.blend'), compress=True)
record = {'status': 'Private state candidate; source and actual-material review pending',
          'geometry_preserved': True, 'outside_appearance_preserved': True,
          'common_receivers': sorted(targets), 'common_material_transfer': 'Identical local mesh UV and material data',
          'source_models': {str(p / 'model.blend'): sha(p / 'model.blend')
                            for p in (args.cover_bake, args.common_bake)},
          'baked_model_sha256': sha(args.output / 'model.blend')}
(args.output / 'assembly.json').write_text(json.dumps(record, indent=2) + '\n')
render(args.manifest, args.output / 'actual', width=384)
print(args.output)
