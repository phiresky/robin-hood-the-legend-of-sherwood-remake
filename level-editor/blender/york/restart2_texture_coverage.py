"""Inspect explicit texture provenance on an unchanged private York bake."""
import argparse
import json
from pathlib import Path
import sys
ROOT=Path(__file__).resolve().parents[3]
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('experiment',type=Path)
parser.add_argument('bake',type=Path)
args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
sys.path.insert(0,str(ROOT/'level-editor/blender/nottingham'))
from freeze_tooling import select_tooling
select_tooling(json.loads((ROOT/'level-editor/work/york-refinement/tooling/current.json').read_text())['directory'])
from render_texture_coverage import inspect
worker=args.bake/'worker.blend'
if not worker.exists():worker.symlink_to((args.bake/'model.blend').resolve())
if worker.resolve()!=(args.bake/'model.blend').resolve():raise ValueError('Unexpected coverage worker')
result=inspect(args.experiment/'views.json',args.bake,args.bake/'coverage-v1')
print(json.dumps({'views':result['views'],'unverified_materials':result['unverified_materials']}))
