"""Privately export the exact approved tree; preserve all live map files."""
import json
import sys
from pathlib import Path
ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from render_slots import acquire
from texture_staging import validate_texture_handoff, verify_baked_geometry
from export_editor import export_asset_library
from review_evidence import sha
R=ROOT/'level-editor/work/croisement01-refinement/restart2'
case=R/'approved-tree-fills-v1/croisement01-tree-18'
acquire()
h=validate_texture_handoff(case/'review-manifest.json','croisement01-tree-18',case/'texture-handoff-v1/decisions.json',case/'decisions.json')
proof=verify_baked_geometry(h)
out=R/'tree18-integration-v1'
out.mkdir(exist_ok=False)
catalog=json.loads((R/'tree18-v4/catalog.json').read_text())
report=export_asset_library('Croisement01',out/'assets',ROOT/'level-editor/work/croisement01-refinement/baseline/Croisement01.rhp.json',asset_ids=['croisement01-tree-18'],catalog=catalog)
(out/'export-proof.json').write_text(json.dumps(dict(scope='private standalone approved tree export; live map unchanged',geometry=proof,export=report,approved_user_decision_sha256=sha(case/'user-texture-decision.json')),indent=2)+'\n')
print(out,flush=True)
