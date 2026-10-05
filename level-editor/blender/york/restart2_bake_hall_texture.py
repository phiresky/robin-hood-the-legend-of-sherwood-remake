"""Bake a visually checked texture candidate onto exact approved York geometry."""
import argparse
import hashlib
import json
from pathlib import Path
import sys

ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/york-refinement'
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('state',choices=['initial-initial','initial-applied','applied-initial','applied-applied'])
parser.add_argument('experiment',type=Path)
parser.add_argument('generation',type=Path)
parser.add_argument('output',type=Path)
args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
sha=lambda p:hashlib.sha256(Path(p).read_bytes()).hexdigest()
if args.output.exists():raise FileExistsError(args.output)
manifest=args.experiment/'views.json'
approval=json.loads((args.experiment/'approval.json').read_text())
review=json.loads((args.generation/'generation-review.json').read_text())
archive=OUT/'restart2/hall-four-state-review-v1/approval-batch-v3'
model=archive/f'{args.state}.blend'
record=json.loads((archive/'archive.json').read_text())
expected=next(row for row in record['states'] if '-'.join(row['state'])==args.state)
if sha(model)!=expected['sha256']:raise ValueError('Archived state changed')
provenance=approval['approval_provenance']
if sha(provenance['receipt'])!=provenance['sha256'] or provenance['sha256']!=record['receipt_sha256']:
    raise ValueError('User approval receipt changed')
if review.get('state')!=args.state:raise ValueError('Texture state mismatch')
if sha(model)!=approval['saved_model_sha256']:raise ValueError('Approved geometry changed')
if not review['bake_authorized'] or review['asset_id']!=approval['asset_id']:raise ValueError('Candidate not reviewed')
for kind in ('raw','preserved'):
    if sha(args.generation/f'generated-{kind}.png')!=review[f'generated_{kind}_sha256']:
        raise ValueError('Reviewed texture changed')
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
sys.path.insert(0,str(ROOT/'level-editor/blender/nottingham'))
from freeze_tooling import select_tooling
select_tooling(json.loads((OUT/'tooling/current.json').read_text())['directory'])
import bpy
from refinement_workspace import _geometry
from project_reviewed_texture import apply
from render_multiview_asset import render
bpy.ops.wm.open_mainfile(filepath=str(model))
scene=bpy.data.scenes['york Refinement'];bpy.context.window.scene=scene
bpy.context.view_layer.update();scene.render.threads_mode='FIXED';scene.render.threads=2
before={o.name:_geometry(o) for o in scene.objects}
outside={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o.get('asset_group')!=approval['asset_id']}
receiver_scope=json.loads(manifest.read_text()).get('texture_receiver_object_names')
unselected={o.name:_geometry(o,protect_appearance=True) for o in scene.objects
            if receiver_scope is not None and o.get('asset_group')==approval['asset_id'] and o.name not in receiver_scope}
report=apply(manifest,args.generation/'generated-preserved.png',args.output,
             texels_per_unit=2,map_name='york',reconciliation_reference=args.generation/'generated-raw.png')
if before!={o.name:_geometry(o) for o in scene.objects}:raise ValueError('Bake changed geometry')
if outside!={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o.name in outside}:
    raise ValueError('Bake changed outside objects or materials')
if unselected!={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o.name in unselected}:
    raise ValueError('Scoped bake changed common materials')
bpy.ops.wm.save_as_mainfile(filepath=str(args.output/'model.blend'),compress=True)
report.update(geometry_verified=True,outside_objects_preserved=len(outside),
              unselected_common_objects_preserved=len(unselected),
              approved_model_sha256=sha(model),baked_model_sha256=sha(args.output/'model.blend'),
              scope='Private texture candidate; actual rendered review and user texture approval pending')
(args.output/'validation.json').write_text(json.dumps(report,indent=2)+'\n')
render(manifest,args.output/'actual',width=384)
print(args.output)
