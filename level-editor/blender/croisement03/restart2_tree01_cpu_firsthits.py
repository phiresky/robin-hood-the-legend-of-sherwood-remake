"""Check proposed bark spines against private wood and retained bank triangles."""
import json,math,hashlib
from pathlib import Path
import numpy as np
from PIL import Image
from restart2_bank_boundary_ray import boundary_hits
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/croisement03-refinement/restart2';O=B/'tree01-cpu-wood-v1'
def main():
 geometry=O/'geometry.json';data=json.loads(geometry.read_text());bank=B/'bank-whole-geometry-plan-v4/geometry.json';banks=json.loads(bank.read_text());meshes={f'wood-{p["source_obstacle_context"]}':(p['vertices'],p['faces']) for p in data['parts']};meshes.update({f'bank-{k}':(banks[k]['vertices'],banks[k]['faces']) for k in ['52','54']});west=json.loads((B/'bank-continuous-strata-plan-v1/geometry.json').read_text());meshes['bank-53']=(west['vertices'],west['faces']);S=math.sin(math.radians(35));C=math.cos(math.radians(35));ray=np.array([0.,-C,S]);rows=[]
 for part in data['parts']:
  node=part['source_obstacle_context'];mask=np.array(Image.open(B/f'tree01-bark-seed-proposal-v1/building-{node:03}-proposed-bark.png'))>0;fail=[];passed=0
  for y,x in np.argwhere(mask):
   origin=np.array([x,-y/S,0.])+ray*5000;hits=boundary_hits(origin,-ray,meshes);owner=hits[0]['owner'] if hits else None
   if owner==f'wood-{node}':passed+=1
   else:fail.append({'source':[int(x),int(y)],'firsthit':owner})
  rows.append({'source_obstacle_context':node,'tested':int(mask.sum()),'passed':passed,'failures':fail})
 bank_changes=[];bank_meshes={k:v for k,v in meshes.items() if k.startswith('bank-')};seeds=np.array(Image.open(B/'bank-exposed-rock-proposal-v1/candidate-rock-seeds.png'))>0
 for y,x in np.argwhere(seeds):
  origin=np.array([x,-y/S,0.])+ray*5000;before=boundary_hits(origin,-ray,bank_meshes);after=boundary_hits(origin,-ray,meshes)
  if not before or not after or before[0]['owner']!=after[0]['owner']:bank_changes.append({'source':[int(x),int(y)],'before':before[0]['owner'] if before else None,'after':after[0]['owner'] if after else None})
 report={'bank_seed_samples':int(seeds.sum()),'bank_seed_firsthit_changes':bank_changes,'status':'PASS_CPU_PROPOSED_BARK_SEEDS' if all(not r['failures'] for r in rows) and not bank_changes else 'HOLD_CPU_PROPOSED_BARK_SEEDS','wood_sha256':hashlib.sha256(geometry.read_bytes()).hexdigest(),'bank_sha256':hashlib.sha256(bank.read_bytes()).hexdigest(),'rows':rows,'limits':['Only proposed narrow bark spines, not all native visible bark.','CPU triangles include52/53/54 bank, not full saved context or foliage.','Saved model, shared crest, neighboring tree and actual eight-view inspection still required.','No material ownership or geometry approval implied.']};(O/'cpu-firsthits.json').write_text(json.dumps(report,indent=2)+'\n');print({'status':report['status'],'tested':sum(r['tested'] for r in rows),'passed':sum(r['passed']for r in rows),'bank_seed_changes':len(bank_changes)})
if __name__=='__main__':main()
