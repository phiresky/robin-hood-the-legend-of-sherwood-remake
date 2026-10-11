"""Fair the inferred upper/lower trunk join without moving bank contact vertices."""
import shutil
import hashlib,json,sys
from pathlib import Path
import numpy as np
from scipy.sparse import coo_matrix,diags
from PIL import Image
from restart2_tree08_local_fork import audit
R=Path(__file__).resolve().parents[3]/'level-editor/work/croisement01-refinement/restart2'
taubin='--taubin' in sys.argv
source=R/'tree08-implicit-collar-v13-smooth-clean/candidate.npz';out=R/('tree08-implicit-collar-v15-taubin-seam' if taubin else 'tree08-implicit-collar-v14-faired-seam');assert not out.exists()
assert shutil.disk_usage(R).free>=10*1024**3
assert next(int(line.split()[1]) for line in Path('/proc/meminfo').read_text().splitlines() if line.startswith('MemAvailable:'))>=6*1024**2
a=np.load(source);v,f=a['vertices'],a['faces'];s,c=np.sin(np.radians(35)),np.cos(np.radians(35));native=-v[:,1]*s-v[:,2]*c
core=np.asarray(Image.open(R/'tree08-semantic-source-v1/bark-core-proposal.png'))>0;y,x=np.where(core)
assert not np.any((y+11>=290)&(y+11<=350)&(x+331>=520)&(x+331<=610))
def smooth(t):
 t=np.clip(t,0,1);return t*t*(3-2*t)
weight=smooth((native-290)/15)*smooth((350-native)/15)*smooth((v[:,0]-520)/15)*smooth((610-v[:,0])/15)
edges=np.concatenate([f[:,[0,1]],f[:,[1,2]],f[:,[2,0]]]);edges=np.concatenate([edges,edges[:,::-1]]);adj=coo_matrix((np.ones(len(edges)),(edges[:,0],edges[:,1])),shape=(len(v),len(v))).tocsr();adj.data[:]=1;adj=diags(1/np.asarray(adj.sum(1)).ravel())@adj
vertices=v.copy()
for i in range(30 if taubin else 160):
 vertices+=(adj@vertices-vertices)*(.5*weight[:,None])
 if taubin:vertices+=(adj@vertices-vertices)*(-.53*weight[:,None])
assert np.array_equal(vertices[native>=350],v[native>=350]);check=audit(vertices,f);out.mkdir();np.savez_compressed(out/'candidate.npz',vertices=vertices,faces=f)
report=dict(status='PRIVATE_INFERRED_SEAM_TRIAL_RECHECK_REQUIRED',source_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),candidate_sha256=hashlib.sha256((out/'candidate.npz').read_bytes()).hexdigest(),iterations=(30 if taubin else 160),rate=.5,reverse_rate=(-.53 if taubin else None),moved_vertices=int(np.count_nonzero(weight)),maximum_displacement=float(np.linalg.norm(vertices-v,axis=1).max()),retained_native_y_350_onward_exact=True,owned_source_pixels_in_seam_region=0,topology=check)
(out/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2),flush=True)
