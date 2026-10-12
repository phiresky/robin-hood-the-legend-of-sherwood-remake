"""Wait outside the shared lease until this bounded audit can enter safely."""
import time,json,hashlib
from pathlib import Path
import clearance_cells_v4 as job
from resource_guard import check
from render_slots import acquire,release

def available():
 return int(next(x.split()[1]for x in Path('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')))*1024
if __name__=='__main__':
 while True:
  while available()<14*1024**3:
   print('Waiting for 14 GiB available memory before shared queue',available(),flush=True);time.sleep(30)
  acquire(slots=2)
  try:
   if available()<10*1024**3:
    print('Admission memory changed; releasing and requeueing',flush=True);continue
   check(job.q.ROOT,job.DEST)
   job.main();break
  finally:release()
