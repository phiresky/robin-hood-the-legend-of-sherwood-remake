"""Export the exact approved stones privately after geometry verification."""
import json
import sys
from pathlib import Path
ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
from render_slots import acquire, release
from texture_staging import validate_texture_handoff, verify_baked_geometry
from export_editor import export_asset_library
from review_evidence import sha
R = ROOT / 'level-editor/work/croisement01-refinement/restart2'
case = R / 'approved-rock29-fill-v2/croisement01-small-bank-stones'
acquire()
try:
    handoff = validate_texture_handoff(case / 'review-manifest.json', 'croisement01-small-bank-stones',
        case / 'texture-handoff-v2/decisions.json', case / 'decisions.json')
    proof = verify_baked_geometry(handoff)
    out = R / 'rock29-integration-v1'
    out.mkdir(exist_ok=False)
    catalog = json.loads((R / 'rock29-v4/catalog.json').read_text())
    report = export_asset_library('Croisement01', out / 'assets',
        ROOT / 'level-editor/work/croisement01-refinement/baseline/Croisement01.rhp.json',
        asset_ids=['croisement01-small-bank-stones'], catalog=catalog)
    (out / 'export-proof.json').write_text(json.dumps(dict(
        scope='Private approved rock export; live map unchanged', geometry=proof, export=report,
        approved_user_decision_sha256=sha(case / 'user-texture-decision.json')), indent=2) + '\n')
    print(out, flush=True)
finally:
    release()
