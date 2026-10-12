"""Root-authorized11GiB prequeue buffer; retain10GiB allocation guards."""
import sys,time,runpy
from pathlib import Path
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[2]/'refinement')]
from render_slots import acquire,release
ALLOWED={'joined02_source_roles_v2.py','joined02_source_roles_v3_all_receivers.py','support04_sweep_v3.py','support04_missing_routes_v1.py'}
if __name__=='__main__':
    name=sys.argv[1];assert name in ALLOWED
    def available():return int(next(x.split()[1]for x in Path('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')))*1024
    while True:
        while available()<11*1024**3:print('Waiting11GiB before bounded job queue',available(),flush=True);time.sleep(30)
        acquire(slots=2)
        try:
            if available()<10*1024**3:continue
            runpy.run_path(str(HERE/name),run_name='__main__');break
        finally:release()
