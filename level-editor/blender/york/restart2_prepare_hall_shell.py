"""Create a private covered-source packet for the saved hall shell control."""
import argparse
import json
from pathlib import Path
import sys

ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/york-refinement'
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--version',required=True)
args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
worker=OUT/'restart2'/args.version
destination=worker/'covered-workspace'
if destination.exists():raise FileExistsError(destination)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
sys.path.insert(0,str(ROOT/'level-editor/blender/nottingham'))
from freeze_tooling import select_tooling
select_tooling(json.loads((OUT/'tooling/current.json').read_text())['directory'])
import bpy
from refinement_workspace import prepare
bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'))
prepare(destination,asset_id='york-castle-great-hall',scene_name='york Refinement',collection_name='york Working',
        source_path=OUT/'baseline/covered.png',grouping_manifest=ROOT/'level-editor/refinement/catalogs/york.json',
        inventory_path=OUT/'inventory/inventory.json',review_path=OUT/'geometry-pass-01/grouping-reconciliation.json')
(destination/'diagnostic-scope.json').write_text(json.dumps({'status':'HOLD','scope':'Covered source first-hit diagnostic only; exact covered/revealed source masks and state-specific receivers remain pending.'},indent=2)+'\n')
