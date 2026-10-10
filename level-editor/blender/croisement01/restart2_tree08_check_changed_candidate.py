"""Resume bounded strict checks against the fully proved connected baseline."""
from concurrent.futures import ThreadPoolExecutor,as_completed
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import numpy as np

ROOT=Path(__file__).resolve().parents[3]
R=ROOT/'level-editor/work/croisement01-refinement/restart2'
packet=R/sys.argv[1]
assert packet.resolve().parent==R.resolve()
baseline=R/'tree08-connected-tip-cleanup-v2'
proof_path=baseline/'intersection-guard-fast.json'
proof=json.loads(proof_path.read_text())
assert proof['status']=='PASS_NO_INTERSECTIONS'
assert proof['candidate_sha256']==hashlib.sha256((baseline/'candidate.npz').read_bytes()).hexdigest()
data=np.load(packet/'candidate.npz');original=np.load(baseline/'candidate.npz')
assert np.array_equal(data['before_vertices'],original['vertices'])
assert np.array_equal(data['faces'],original['faces'])
changed=np.any(np.any(data['vertices'][data['faces']]!=data['before_vertices'][data['faces']],axis=2),axis=1)
count=(int(changed.sum())+1999)//2000
(packet/'baseline-proof.json').write_text(json.dumps(dict(unchanged_geometry_proof=str(proof_path),proof_sha256=hashlib.sha256(proof_path.read_bytes()).hexdigest(),unchanged_geometry_exact=True,changed_faces=int(changed.sum()),chunks=count),indent=2)+'\n')
def run(index):
    output=packet/f'intersection-chunks-v1/chunk-{index:03d}.json'
    if output.exists():return index
    print('CHUNK',index,'OF',count,flush=True)
    with (packet/f'guard-{index:03d}.log').open('w') as log:
        subprocess.run([sys.executable,str(Path(__file__).with_name('restart2_tree08_root_ray_guard_chunked.py')),'--packet',packet.name,'--chunk',str(index)],stdout=log,stderr=subprocess.STDOUT,check=True)
    return index
with ThreadPoolExecutor(max_workers=2) as pool:
    for future in as_completed([pool.submit(run,index) for index in range(count)]):
        print('FINISHED',future.result(),flush=True)
result=json.loads((packet/'intersection-guard.json').read_text())
print(result['status'],'new',len(result['new_intersections']),'retained',len(result['retained_intersections']),flush=True)
