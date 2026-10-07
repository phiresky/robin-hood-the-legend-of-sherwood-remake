"""Prepare the approved recessed jamb with its reviewed native pixel domain."""
import hashlib
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
WORK = ROOT / 'level-editor/work/york-refinement/restart2'
OUT = WORK / 'jamb-texture-inputs-v1'
PROBE = WORK / 'jamb-source-probe-v1'
GEO = WORK / 'gate-geometry-v10'
OBJECT = 'building-778-portcullis-jamb-return'
ASSET = 'york-castle-west-gatehouse'
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
if OUT.exists():
    raise FileExistsError(OUT)
approval = json.loads((GEO / 'user-geometry-approval.json').read_text())
assert sha(Path(approval['receipt'])) == approval['receipt_sha256']
member = next(m for m in approval['members'] if m['asset_id'].endswith('--portcullis-jamb-return'))
model = Path(member['model'])
assert sha(model) == member['model_sha256']
probe = json.loads((PROBE / 'report.json').read_text())
assert probe['approved_model_sha256'] == sha(model)
assert probe['visible_pixels'] == 660
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
from PIL import Image
sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
from refinement_review import render_review
from refinement_workspace import _geometry

bpy.ops.wm.open_mainfile(filepath=str(model))
scene = bpy.context.scene
bpy.context.view_layer.update()
before = {o.name: _geometry(o, protect_appearance=True) for o in scene.objects}
receiver = scene.objects[OBJECT]
assert receiver.get('source_node') == 'building-778'
assert receiver.get('projection_component') == 'portcullis-jamb-return'
OUT.mkdir(parents=True)
source = Image.new('RGBA', (2500, 1100))
source.alpha_composite(Image.open(WORK / 'gate-source-study-v2/initial.png').convert('RGBA'), (2250, 780))
source.save(OUT / 'source.png')
domain = Image.open(PROBE / 'visible-jamb-domain.png').convert('L')
assert sum(v > 0 for v in domain.getdata()) == 660
domain.save(OUT / 'known-domain.png')
inventory = {'version': 1, 'index_namespace': 'Reviewed authored jamb domain, not native mask ID', 'masks': [
    {'index': 0, 'kind': 'authored-artwork-domain', 'box_top_left': [2250, 780], 'box_size': [220, 250], 'png': 'known-domain.png'}]}
(OUT / 'inventory.json').write_text(json.dumps(inventory, indent=2) + '\n')
authority = {
    'status': 'Root reviewed native overlay and enlarged source; exact colors accepted for recessed return',
    'material_inference': 'Dark masonry suggested by faint horizontal courses, not categorical proof at native resolution. Hidden faces infer matching dark stone from the same arch.',
    'exclusions': ['Moving gate bars', 'Bright patch000 interior', 'Original building-778 context', 'Winch and room'],
    'receiver_object': OBJECT, 'source_node': 'building-778', 'projection_component': 'portcullis-jamb-return',
    'accepted_pixels': 660, 'domain_sha256': sha(OUT / 'known-domain.png'),
    'source_sha256': sha(OUT / 'source.png'), 'approved_model_sha256': sha(model),
    'approval_receipt_sha256': approval['receipt_sha256'],
}
(OUT / 'authority.json').write_text(json.dumps(authority, indent=2) + '\n')
manifest = {'version': 1, 'mask_inventory': str((OUT / 'inventory.json').resolve()), 'projections': {
    'jamb-native': {'source_sha256': sha(OUT / 'source.png'), 'assignments': [
        {'reviewed': True, 'asset_group': ASSET, 'mask_indices': [0], 'review_evidence': str((OUT / 'authority.json').resolve())}]}}}
(OUT / 'source-masks.json').write_text(json.dumps(manifest, indent=2) + '\n')
collection = bpy.data.collections.new('Jamb texture context')
scene.collection.children.link(collection)
for o in scene.objects:
    if o.type == 'MESH':
        collection.objects.link(o)
nodes = sorted({o.get('source_node') for o in collection.objects})
layer = {'source_path': str((OUT / 'source.png').resolve()), 'projection_label': 'jamb-native',
         'receiver_nodes': ['building-778'], 'occluder_nodes': nodes}
render_review(OUT / 'modified', scene_name=scene.name, collection_name=collection.name,
              asset_id=ASSET, source_path=OUT / 'source.png', width=384, height=512,
              source_mask_manifest=OUT / 'source-masks.json', projection_layers=[layer],
              render_object_names=[OBJECT])
assert before == {o.name: _geometry(o, protect_appearance=True) for o in scene.objects if o.name in before}
(OUT / 'input-review.json').write_text(json.dumps({
    'status': 'Visual input review pending; no provider request',
    'geometry_uv_materials_preserved': True, 'approved_model_sha256': sha(model),
    'texture_receiver_object_names': [OBJECT], 'views_sha256': sha(OUT / 'modified/views.json'),
    'authority_sha256': sha(OUT / 'authority.json'),
}, indent=2) + '\n')
print('JAMB INPUTS COMPLETE', flush=True)
