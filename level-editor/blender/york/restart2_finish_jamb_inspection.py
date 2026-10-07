"""Inspect the saved jamb bake without repeating synthesis or baking."""
import hashlib
import json
from pathlib import Path
import runpy
import sys

ROOT = Path(__file__).resolve().parents[3]
WORK = ROOT / 'level-editor/work/york-refinement/restart2'
BAKE = WORK / 'jamb-textures-v1'
EXPERIMENT = WORK / 'jamb-texture-inputs-v3/experiment'
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
validation = json.loads((BAKE / 'validation.json').read_text())
assert sha(BAKE / 'model.blend') == validation['baked_model_sha256']
original = WORK / 'jamb-texture-inputs-v3/modified/views.json'
prepared = json.loads((EXPERIMENT / 'views.json').read_text())
source = json.loads(original.read_text())
assert len(prepared['views']) == len(source['views']) == 8
for a, b in zip(prepared['views'], source['views']):
    assert a['camera_matrix_world'] == b['camera_matrix_world']
    assert a['ortho_scale'] == b['ortho_scale']
sys.path.insert(0, str(Path(__file__).parent))
from restart2_jamb_actual_sheet import assemble
assemble(BAKE / 'actual')
from restart2_camera_audit import audit_manifest, labeled_copy
audit = audit_manifest(original)
audit['prepared_manifest_sha256'] = sha(EXPERIMENT / 'views.json')
audit['prepared_cameras_exact'] = True
(BAKE / 'native-camera-audit.json').write_text(json.dumps(audit, indent=2) + '\n')
labeled_copy(BAKE / 'actual/textured.png', BAKE / 'actual/textured-native-labeled.png')
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
from render_slots import acquire
acquire()
sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
coverage_input = BAKE / 'coverage-input'
coverage_input.mkdir()
(coverage_input / 'worker.blend').symlink_to((BAKE / 'model.blend').resolve())
for layer in BAKE.glob('layer-*.json'):
    (coverage_input / layer.name).symlink_to(layer.resolve())
from render_texture_coverage import inspect
inspect(EXPERIMENT / 'views.json', coverage_input, BAKE / 'coverage-v1')
runpy.run_path(str(Path(__file__).with_name('restart2_jamb_texture_contact.py')), run_name='__main__')
assert sha(BAKE / 'model.blend') == validation['baked_model_sha256']
print('JAMB INSPECTION COMPLETE', flush=True)
