"""Render private furniture solids without claiming revealed texture ownership."""
import json
import argparse
from pathlib import Path
import sys

ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/york-refinement'
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--version',required=True)
args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
DEST=OUT/'restart2'/args.version/'solid-inspection'
if DEST.exists():raise FileExistsError(DEST)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
sys.path.insert(0,str(ROOT/'level-editor/blender/nottingham'))
from freeze_tooling import select_tooling
select_tooling(json.loads((OUT/'tooling/current.json').read_text())['directory'])
import bpy
from refinement_review import render_review
bpy.ops.wm.open_mainfile(filepath=str(DEST.parent/'model.blend'))
config=json.loads((OUT/'geometry-pass-01/assets/york-castle-great-hall/workspace.json').read_text())
for obj in bpy.data.collections['york Working'].all_objects:
    if obj.type=='MESH' and obj.get('source_node') in {f'building-{n}' for n in range(824,830)}:
        obj['asset_group']='private-york-hall-furniture'
render_review(DEST,scene_name='york Refinement',collection_name='york Working',
              asset_id='private-york-hall-furniture',source_path=OUT/'baseline/revealed.png',
              width=384,height=512,framing_padding=1.5,lighting=config['lighting'])
(DEST/'diagnostic-scope.json').write_text(json.dumps({'scope':'Solid geometry inspection only. Any helper source-textured output is not state ownership evidence.',
    'saved_model_unchanged':True,'approval':False},indent=2)+'\n')
