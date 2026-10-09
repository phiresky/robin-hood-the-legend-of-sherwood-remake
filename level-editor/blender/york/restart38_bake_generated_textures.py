"""Bake a visually checked texture candidate onto exact approved York well or stable winch components."""
import argparse
import hashlib
import json
from pathlib import Path
import sys
import shutil
assert shutil.disk_usage(Path.cwd()).free>10*1024**3
assert int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')))*1024>6*1024**3

ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/york-refinement'
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('experiment',type=Path)
parser.add_argument('generation',type=Path)
parser.add_argument('output',type=Path)
args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
sha=lambda p:hashlib.sha256(Path(p).read_bytes()).hexdigest()
if args.output.exists():raise FileExistsError(args.output)
manifest=args.experiment/'views.json'
approval=json.loads((args.experiment/'approval.json').read_text())
review=json.loads((args.generation/'generation-review.json').read_text())
assert approval['asset_id'] in ('york-market-roofed-stone-well','york-castle-winch','york-riverside-stone-storehouse','york-castle-west-gatehouse--guide-interiors')
input_review=json.loads((args.experiment.parent/'input-review.json').read_text())
receiver_names=set(input_review['object_names'])
model=args.experiment/'approved-model.blend'
if sha(model)!=approval['saved_model_sha256']:raise ValueError('Approved geometry changed')
if not review['bake_authorized'] or review['asset_id']!=approval['asset_id']:raise ValueError('Candidate not reviewed')
for kind in ('raw','preserved'):
    if sha(args.generation/f'generated-{kind}.png')!=review[f'generated_{kind}_sha256']:
        raise ValueError('Reviewed texture changed')
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire(slots=2)
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
import bpy
from refinement_workspace import _geometry
from project_reviewed_texture import apply
from render_multiview_asset import render
bpy.ops.wm.open_mainfile(filepath=str(model))
scene=bpy.context.scene
if args.experiment.parent.name=='stable24':
    freeze=json.loads((Path(input_review['approved_scope']['model']).parent/'component-freeze.json').read_text())
    scene.frame_set(freeze['poses'][44]['tick'])
collection=bpy.data.collections.new('Approved York texture context');scene.collection.children.link(collection)
for o in scene.objects:
    if o.type=='MESH':collection.objects.link(o)
bpy.context.view_layer.update();scene.render.threads_mode='FIXED';scene.render.threads=2
before={o.name:_geometry(o) for o in scene.objects}
outside={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o.name not in receiver_names}
report=apply(manifest,args.generation/'generated-preserved.png',args.output,
             texels_per_unit=2,map_name='york',reconciliation_reference=args.generation/'generated-raw.png')
if before!={o.name:_geometry(o) for o in scene.objects}:raise ValueError('Bake changed geometry')
if outside!={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o.name in outside}:
    raise ValueError('Bake changed outside objects or materials')
# Keep only the reviewed receiver in the saved worker; context was checked above.
receivers=[o for o in scene.objects if o.type=='MESH' and o.name in receiver_names]
scoped_before={o.name:_geometry(o,protect_appearance=True) for o in receivers}
keep=set(receivers)
for o in receivers:
    ancestor=o.parent
    while ancestor is not None:keep.add(ancestor);ancestor=ancestor.parent
for o in list(bpy.data.objects):
    if o not in keep:bpy.data.objects.remove(o,do_unlink=True)
for blocks in (bpy.data.meshes,bpy.data.materials,bpy.data.images):
    for block in list(blocks):
        if block.users==0:blocks.remove(block)
bpy.context.view_layer.update()
assert scoped_before=={o.name:_geometry(o,protect_appearance=True) for o in receivers}
bpy.context.preferences.filepaths.save_version=0
bpy.ops.wm.save_as_mainfile(filepath=str(args.output/'model.blend'),compress=True)
assert (args.output/'model.blend').stat().st_size<64*1024**2
report.update(geometry_verified=True,outside_objects_preserved=len(outside),
              approved_model_sha256=sha(model),baked_model_sha256=sha(args.output/'model.blend'),
              scope='Private texture candidate; actual rendered review and user texture approval pending')
(args.output/'validation.json').write_text(json.dumps(report,indent=2)+'\n')
bpy.ops.wm.open_mainfile(filepath=str(args.output/'model.blend'))
render(manifest,args.output/'actual',width=320)
assert sum(p.stat().st_size for p in args.output.rglob('*') if p.is_file())<64*1024**2
print(args.output)
