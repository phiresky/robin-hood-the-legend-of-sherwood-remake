"""Independent first-hit guards for fixed sidewall traces and bank source seeds."""
import json,math,hashlib
from pathlib import Path
import numpy as np
from PIL import Image
from restart2_bank_boundary_ray import boundary_hits
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/croisement03-refinement/restart2';O=B/'bank-sidewall-cpu-v1';S=math.sin(math.radians(35));C=math.cos(math.radians(35));ray=np.array([0.,-C,S])
def main():
 old=json.loads((B/'bank-whole-geometry-plan-v4/geometry.json').read_text());new=json.loads((O/'geometry.json').read_text());west=json.loads((B/'bank-continuous-strata-plan-v1/geometry.json').read_text());old['53']=new['53']=west;meshes=lambda d:{k:(d[k]['vertices'],d[k]['faces'])for k in ['52','53','54']};before=meshes(old);after=meshes(new);rows=json.loads((B/'bank-shoulder-face-routing-v1/routing.json').read_text())['rows'];control_errors=[]
 for row in rows:
  if row['surface']!='side':continue
  x,y=row['source'];origin=np.array([x,-y/S,0.])+ray*5000;hits=boundary_hits(origin,-ray,after);expected=np.array(row['world']);error=float(np.linalg.norm(origin-ray*hits[0]['distance']-expected))if hits else None
  if not hits or hits[0]['owner']!=row['firsthit_owner']or error>1e-4:control_errors.append({'source':row['source'],'error':error})
 seeds=np.array(Image.open(B/'bank-exposed-rock-proposal-v1/candidate-rock-seeds.png'))>0;changed=[]
 for y,x in np.argwhere(seeds):
  origin=np.array([x,-y/S,0.])+ray*5000;a=boundary_hits(origin,-ray,before);b=boundary_hits(origin,-ray,after)
  if not a or not b or a[0]['owner']!=b[0]['owner']:changed.append([int(x),int(y)])
 report={'status':'PASS_CPU_SIDEWALL_ANCHORS_AND_BANK_SEEDS'if not control_errors and not changed else'HOLD','fixed_sidewall_samples':sum(r['surface']=='side'for r in rows),'control_errors':control_errors,'bank_seed_samples':int(seeds.sum()),'bank_seed_firsthit_changes':changed,'geometry_sha256':hashlib.sha256((O/'geometry.json').read_bytes()).hexdigest(),'limits':['Private local sidewall study; whole52/54 morphology remains incomplete.','Saved Boolean/context/neighbors and native/actual/solid contact review still required.','No source pixel ownership or texture fill assigned.']};(O/'source-firsthit-guard.json').write_text(json.dumps(report,indent=2)+'\n');print({k:len(v)if isinstance(v,list)else v for k,v in report.items()if k!='limits'})
if __name__=='__main__':main()
