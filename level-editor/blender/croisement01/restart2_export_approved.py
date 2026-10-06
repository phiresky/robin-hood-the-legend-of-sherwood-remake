"""Export approved single-asset texture candidates privately with geometry guards."""
import json
import sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from render_slots import acquire,release
from texture_staging import validate_texture_handoff,verify_baked_geometry
from export_editor import export_asset_library
from review_evidence import sha
R=ROOT/'level-editor/work/croisement01-refinement/restart2'
kind=sys.argv[sys.argv.index('--')+1]
folder,asset={'tree03':('approved-tree03-fill-v1','croisement01-tree-03'),'tree19':('approved-tree19-fill-v1','croisement01-tree-19'),'tree71':('approved-tree71-fill-v1','croisement01-tree-71'),'stump64':('approved-stump64-wood-fill-v1','croisement01-southwest-broken-stump'),'stump67':('approved-stump67-wood-fill-v1','croisement01-east-ivy-stump'),'tree22':('approved-tree22-fill-v1','croisement01-tree-22'),'tree21':('approved-tree21-fill-v1','croisement01-tree-21'),'stump65':('approved-stump65-wood-fill-v1','croisement01-southwest-cut-stump'),'tree20':('approved-tree-fills-v1','croisement01-tree-20'),'stump68':('approved-stump68-wood-fill-v1','croisement01-southeast-small-stump')}[kind]
case=R/folder/asset
acquire()
try:
 h=validate_texture_handoff(case/'review-manifest.json',asset,case/'texture-handoff-v1/decisions.json',case/'decisions.json')
 proof=verify_baked_geometry(h)
 grouping=R/({'tree03':'tree03-v4','tree19':'tree19-v6','tree71':'tree71-v5','stump64':'stump64-wood-v4','stump67':'stump67-wood-v7','tree22':'tree22-v8','tree21':'tree21-v7','stump65':'stump65-wood-fit-v2','tree20':'tree20-v3','stump68':'stump68-wood-fit-v2'}[kind])/'assets'/asset/'reference/grouping.json'
 config=json.loads(((R/'tree03-v4/assets'/asset/'workspace.json') if kind=='tree03' else (case/'approved-workspace/workspace.json')).read_text())
 assert sha(grouping)==config['grouping_manifest_sha256']
 catalog=json.loads(grouping.read_text())
 out=R/(kind+'-integration-v2');out.mkdir(exist_ok=False)
 report=export_asset_library('Croisement01',out/'assets',ROOT/'level-editor/work/croisement01-refinement/baseline/Croisement01.rhp.json',asset_ids=[asset],catalog=catalog)
 (out/'export-proof.json').write_text(json.dumps(dict(scope='Private exact approved asset export; live map unchanged',geometry=proof,export=report,approved_user_decision_sha256=sha(case/'user-texture-decision.json')),indent=2)+'\n')
 print(out,flush=True)
finally:release()
