"""Reproduce approved hall cameras with reviewed state/component source ownership."""
import argparse
import hashlib
import json
from pathlib import Path
import sys
ROOT=Path(__file__).resolve().parents[3]
BASE=ROOT/'level-editor/work/york-refinement'
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('state',choices=['initial-initial','initial-applied','applied-initial','applied-applied'])
args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
output=BASE/'restart2/hall-textures-v1'/args.state/'worker'
if output.exists():raise FileExistsError(output)
archive=BASE/'restart2/hall-four-state-review-v1/approval-batch-v3'
sha=lambda p:hashlib.sha256(Path(p).read_bytes()).hexdigest()
receipt=archive/'user-approval.json'
if sha(receipt)!='74dbd2648ba2a9d54902cf89c1fd05b7d98a8e1da8d311f4ead227f1fbcc2ee9':raise ValueError('Hall approval receipt changed')
model=archive/(args.state+'.blend')
decision=next(v for v in json.loads(receipt.read_text())['decisions'] if v['asset_id']=='york-castle-great-hall--'+args.state)
if decision['decision']!='approved' or decision['scope']!='geometry' or decision['model_sha256']!=sha(model):raise ValueError('Hall state not approved')
state=next(v for v in json.loads((archive/'ready-candidate-v1.json').read_text())['states'] if v['patch001']+'-'+v['patch002']==args.state)
workspace=ROOT/state['workspace'];frames_path=workspace/'inspection-v1/complete-object/views.json';frames=json.loads(frames_path.read_text())
mask=BASE/'restart2/hall-source-authority-v1/source-masks.json'
layers=[{**r,'projection_label':'hall-semantic-'+args.state} for r in frames['projection_layers']]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
sys.path.insert(0,str(ROOT/'level-editor/blender/nottingham'))
from freeze_tooling import select_tooling
select_tooling(json.loads((BASE/'tooling/current.json').read_text())['directory'])
import bpy
import numpy as np
from refinement_review import render_review
from refinement_workspace import _geometry
bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.data.scenes['york Refinement'];bpy.context.window.scene=scene
bpy.context.view_layer.update();scene.render.threads_mode='FIXED';scene.render.threads=2
before={o.name:_geometry(o,protect_appearance=True) for o in scene.objects}
output.mkdir(parents=True);(output/'model.blend').symlink_to(model)
config=json.loads((workspace/'workspace.json').read_text());(output/'workspace.json').write_text(json.dumps(config,indent=2)+'\n')
packet=render_review(output/'modified',scene_name=frames['scene_name'],collection_name=frames['collection_name'],
    asset_id='york-castle-great-hall',source_path=frames['source_image'],frame_manifest=frames,
    source_mask_manifest=mask,projection_layers=layers,allow_projection_revision=True,allow_mask_revision=True,
    lighting=frames['lighting'],render_object_names=frames['object_names'])
if before!={o.name:_geometry(o,protect_appearance=True) for o in scene.objects}:raise ValueError('Texture input changed approved hall or neighbor appearance')
for a,b in zip(packet['views'],frames['views']):
    if np.max(np.abs(np.array(a['camera_matrix_world'])-np.array(b['camera_matrix_world'])))>1e-5 or abs(a['ortho_scale']-b['ortho_scale'])>1e-5:raise ValueError('Approved camera changed')
if packet['tile_size']!=frames['tile_size']:raise ValueError('Approved resolution changed')
(output/'input-review.json').write_text(json.dumps({'status':'Unchanged approved state; visual known-source input review pending',
    'asset_id':'york-castle-great-hall','state':args.state,'decision_id':decision['asset_id'],
    'model_sha256':sha(model),'user_receipt_sha256':sha(receipt),'approved_camera_sha256':sha(frames_path),
    'source_authority_sha256':sha(mask),'geometry_uv_materials_preserved':True,'native_first_cameras_preserved':True,
    'source_scope':'Furniture/candle native masks and traced stone arch ring are separate from shell; keep doorway and adjacent towers excluded.',
    'texture_approval':'none'},indent=2)+'\n');print(output)
