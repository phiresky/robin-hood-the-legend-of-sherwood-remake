"""Locate serialization depth outliers without changing acceptance thresholds."""
from pathlib import Path
import json,numpy as np
from PIL import Image
from restart2_tree08_remaining_forks import inputs
from restart2_tree08_junction_proof import native_depth
R=Path(__file__).resolve().parents[2]/'work/croisement01-refinement/restart2';p=R/'tree08-v12-remaining-group0-stitched-v3-conformed-stable';m=np.load(p/'candidate.npz');v=m['vertices'];f=m['faces'];origin=np.array([552.,-672.,235.]);saved=(v-origin).astype(np.float32).astype(np.float64)+origin
meshes,_,groups,offset=inputs(R);reference=native_depth([(meshes[i][0]+offset,meshes[i][1]) for i in groups[0]]);current=native_depth([(saved,f)]);valid=np.isfinite(reference)&np.isfinite(current);delta=np.zeros_like(reference);delta[valid]=abs(reference[valid]-current[valid]);core=np.asarray(Image.open(R/'tree08-semantic-source-v1/bark-core-proposal.png'))>0
out=[]
for yy,xx in np.argwhere(delta>2e-4):out.append(dict(native=[int(xx)+331,int(yy)+11],reference=float(reference[yy,xx]),saved=float(current[yy,xx]),delta=float(delta[yy,xx]),core=bool(core[yy,xx])))
report=dict(status='Diagnostic only;2e-4guard unchanged',outliers=sorted(out,key=lambda x:-x['delta']),maximum_core_depth_difference=float(delta[core].max()),maximum_difference=float(delta.max()));(p/'depth-outliers.json').write_text(json.dumps(report,indent=2)+'\n');print(report)
