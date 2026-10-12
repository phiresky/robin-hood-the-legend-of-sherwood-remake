"""Run a checkpointed receiver-depth envelope under the shared resource budget."""
import os,sys,runpy,shutil
from pathlib import Path
os.environ['OPENBLAS_NUM_THREADS']='2';os.environ['OMP_NUM_THREADS']='2'
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'));from render_slots import acquire,release
acquire(2)
try:
 mem={k:int(v.split()[0])*1024 for k,v in (line.split(':',1) for line in Path('/proc/meminfo').read_text().splitlines())};assert mem['MemAvailable']>=10*1024**3;assert shutil.disk_usage(ROOT).free>=10*1024**3
 runpy.run_path(str(Path(__file__).with_name('restart94_own_butterfly_depth_envelope.py')),run_name='__main__')
finally:release()
