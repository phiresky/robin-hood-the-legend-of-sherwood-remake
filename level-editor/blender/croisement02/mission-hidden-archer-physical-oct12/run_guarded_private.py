"""Resource-gated launcher for named private archer evidence jobs."""
import sys,time,runpy
from pathlib import Path
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[2]/'refinement')]
from render_slots import acquire,release
ALLOWED={'contact_cell_conformed_v5.py','support_route_probe_v1.py'}
if __name__=='__main__':
 name=sys.argv[1];assert name in ALLOWED
 def available():return int(next(x.split()[1]for x in Path('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')))*1024
 while True:
  while available()<14*1024**3:print('Waiting14GiB before shared queue',available(),flush=True);time.sleep(30)
  acquire(slots=2)
  try:
   if available()<10*1024**3:continue
   runpy.run_path(str(HERE/name),run_name='__main__');break
  finally:release()
