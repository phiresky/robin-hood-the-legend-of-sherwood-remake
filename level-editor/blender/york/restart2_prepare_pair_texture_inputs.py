"""Render approved York pair geometry with its reviewed semantic source masks."""
import argparse
import hashlib
import json
from pathlib import Path
import sys

ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/york-refinement'
PAIR=OUT/'restart2/pair-v16'
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('asset',choices=['york-market-southeast-tall-narrow-house','york-southwest-square-west-house'])
parser.add_argument('output',type=Path)
args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
if args.output.exists():raise FileExistsError(args.output)
sha=lambda p:hashlib.sha256(Path(p).read_bytes()).hexdigest()
receipt=ROOT/'level-editor/work/croisement02-refinement/restart3-review-batches/batch-v2/user-approval.json'
if sha(receipt)!='393634eb3e7fd57f946b7aecc029abc255da94c0d8c72ba7c97cab5855ddca63':raise ValueError('User approval receipt changed')
model=PAIR/'approval-batch-v2/pair-materials.blend'
decisions=json.loads(receipt.read_text())['decisions']
decision=next(r for r in decisions if r['asset_id']==args.asset and r['scope']=='geometry')
if decision['decision']!='approved' or decision['model_sha256']!=sha(model):raise ValueError('Approval does not match pair geometry')
label='bay' if 'tall-narrow' in args.asset else 'house'
frames_path=PAIR/f'assembled-review/{label}/inspection-v1/complete-object/views.json'
frames=json.loads(frames_path.read_text())
mask=PAIR/'source-authority-v1/source-masks.json'
layers=[{**r,'projection_label':'pair-reviewed-source'} for r in frames['projection_layers']]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
sys.path.insert(0,str(ROOT/'level-editor/blender/nottingham'))
from freeze_tooling import select_tooling
select_tooling(json.loads((OUT/'tooling/current.json').read_text())['directory'])
import bpy
import numpy as np
from refinement_review import render_review
from refinement_workspace import _geometry
bpy.ops.wm.open_mainfile(filepath=str(model))
scene=bpy.data.scenes['york Refinement'];bpy.context.window.scene=scene
bpy.context.view_layer.update();scene.render.threads_mode='FIXED';scene.render.threads=2
before={o.name:_geometry(o,protect_appearance=True) for o in scene.objects}
args.output.mkdir(parents=True)
(args.output/'model.blend').symlink_to(model)
config=json.loads((PAIR/f'assembled-review/{label}/workspace.json').read_text())
(args.output/'workspace.json').write_text(json.dumps(config,indent=2)+'\n')
packet=render_review(args.output/'modified',scene_name=frames['scene_name'],collection_name=frames['collection_name'],
    asset_id=args.asset,source_path=frames['source_image'],frame_manifest=frames,
    source_mask_manifest=mask,projection_layers=layers,allow_projection_revision=True,
    allow_mask_revision=True,lighting=frames['lighting'],render_object_names=frames['object_names'])
if before!={o.name:_geometry(o,protect_appearance=True) for o in scene.objects}:raise ValueError('Input preparation changed approved scene')
for a,b in zip(packet['views'],frames['views']):
    if np.max(np.abs(np.array(a['camera_matrix_world'])-np.array(b['camera_matrix_world'])))>1e-5 or abs(a['ortho_scale']-b['ortho_scale'])>1e-5:
        raise ValueError('Approved camera changed')
if packet['tile_size']!=frames['tile_size']:raise ValueError('Approved resolution changed')
report={'status':'Unchanged approved geometry; source-mask-constrained texture inputs awaiting visual input review',
    'asset_id':args.asset,'model_sha256':sha(model),'user_receipt_sha256':sha(receipt),
    'approved_camera_sha256':sha(frames_path),'semantic_source_manifest_sha256':sha(mask),
    'geometry_uv_materials_preserved':True,'native_first_cameras_preserved':True,
    'projection_change':'The separately reviewed semantic ownership masks constrain known artwork; uncertain edges remain editable. Geometry, cameras and original source artwork unchanged.',
    'tile_size':packet['tile_size'],'scope':'User geometry approval authorizes texture generation; generated textures still need separate review.'}
(args.output/'input-review.json').write_text(json.dumps(report,indent=2)+'\n')
print(args.output)
