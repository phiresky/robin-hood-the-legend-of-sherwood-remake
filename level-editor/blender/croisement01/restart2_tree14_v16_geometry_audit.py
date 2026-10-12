"""Build one bounded revision and audit its saved file under one render lease."""
import runpy
import sys
from pathlib import Path

HERE=Path(__file__).resolve().parent
ROOT=HERE.parents[2]
sys.argv=['geometry','--','--geometry-only']
try:
    runpy.run_path(str(HERE/'restart2_tree14_upper_wood.py'),run_name='__main__')
except SystemExit as result:
    if result.code not in (None,0):raise
sys.argv=['audit','--','--tree','14','--workspace',str(ROOT/'level-editor/work/active/croisement01/restart2/tree14-wood-v16/assets/croisement01-tree-14'),'--model-name','baseline.blend']
runpy.run_path(str(HERE/'restart2_wood_coverage.py'),run_name='__main__')
