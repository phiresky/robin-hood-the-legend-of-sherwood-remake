"""Run the private browser aperture proof under the shared render pool."""
from pathlib import Path
import sys,subprocess
sys.path.insert(0,str(Path(__file__).resolve().parents[2]/'refinement'))
from render_slots import acquire,release
acquire()
try:
 raise SystemExit(subprocess.call(['node',str(Path(__file__).with_name('restart10_verify_hole_aperture_editor.mjs')),*sys.argv[1:]]))
finally:release()
