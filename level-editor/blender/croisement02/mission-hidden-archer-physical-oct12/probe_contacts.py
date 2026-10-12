"""Sample alpha-aware receiver rays before selecting inferred screen depth."""
import os
os.environ['OPENBLAS_NUM_THREADS']='2'
import sys,json,shutil
from pathlib import Path
import numpy as np
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE.parent),str(HERE.parents[2]/'refinement')]
import qualify as q
import restart14_butterfly_canopy22_audit as reader
from render_slots import acquire,release
DEST=q.OUT/'receiver-probes-v1'
def main():
 assert shutil.disk_usage(q.ROOT).free>=10*1024**3+16*1024**2
 mem=dict(line.split(':',1) for line in Path('/proc/meminfo').read_text().splitlines());assert int(mem['MemAvailable'].split()[0])*1024>=10*1024**3
 assert not DEST.exists();report=json.loads((q.OUT/'qualification.json').read_text());rays=[];assets=set()
 for n,row in enumerate(report['rows'],1):
  assets.update(x['id']for x in row['bounds_candidates'])
  for state in ['initial','transition']:
   f=row['art']['states'][state]['frames'][-1];a=np.array(q.rgba(f))[:,:,3]>0;cols=np.flatnonzero(a.any(0))
   for x in np.unique(np.quantile(cols,[.05,.15,.5,.85,.95]).astype(int)):
    ys=np.flatnonzero(a[:,x]);
    for y in [ys[0],ys[-1]]:rays.append(dict(profile=n,state=state,screen=[f['top_left'][0]+int(x)+.5,f['top_left'][1]+int(y)+.5],hits=[]))
 def finish(rays,records,mp):
  DEST.mkdir();result=dict(map=q.pin(mp),receivers=records,rays=rays,scope='Alpha-aware source rays only, not contact or shape approval. Ground/branch/support association must be inspected before choosing depth.');(DEST/'report.json').write_text(json.dumps(result,indent=2)+'\n');print('PROBES',len(rays),flush=True)
 reader.main(ray_records=rays,postprocess=finish,output=DEST,asset_ids=assets,minimum_free_bytes=10*1024**3,output_limit_bytes=16*1024**2,expected_map_sha256=report['map']['sha256'])
if __name__=='__main__':
 acquire(slots=2)
 try:main()
 finally:release()
