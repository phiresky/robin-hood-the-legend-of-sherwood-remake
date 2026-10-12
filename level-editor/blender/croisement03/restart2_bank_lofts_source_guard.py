"""Independent source rays for joined shoulder lofts and inferred-depth provenance."""
import json,math,hashlib
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
from restart2_bank_boundary_ray import boundary_hits
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/croisement03-refinement/restart2';O=B/'bank-shoulder-lofts-cpu-v6';E=B/'bank-east-loft-cpu-v7/geometry.json';S=math.sin(math.radians(35));C=math.cos(math.radians(35));ray=np.array([0.,-C,S])
def main():
 d=json.loads((O/'geometry.json').read_text());east=json.loads(E.read_text());old=json.loads((B/'bank-whole-geometry-plan-v4/geometry.json').read_text());west=json.loads((B/'bank-continuous-strata-plan-v1/geometry.json').read_text());pair=lambda m:(m['vertices'],m['faces']);before={'52':pair(old['52']),'53':pair(west),'54':pair(old['54'])}
 def after(x):
  meshes={'53':pair(west),'54':pair(d['54']),'52-right':pair(east)}
  if x<=east['x_cut']:meshes['52-left']=pair(d['52'])
  return meshes
 def hits(source,meshes):
  x,y=source;origin=np.array([x,-y/S,0.])+ray*5000;return boundary_hits(origin,-ray,meshes),origin
 failures=[];count=0
 for owner in ['52','54']:
  m=d[owner]
  for trace in m['traces']:
   points=np.array(m['vertices'])[trace['vertices']]
   for a,b in zip(points,points[1:]):
    distance=np.linalg.norm([b[0]-a[0],-(b[1]-a[1])*S-(b[2]-a[2])*C])
    for t in np.linspace(0,1,max(2,int(distance*2)+1)):
     p=a*(1-t)+b*t;source=[float(p[0]),float(-p[1]*S-p[2]*C)];hs,origin=hits(source,after(p[0]));error=float(np.linalg.norm(origin-ray*hs[0]['distance']-p))if hs else None;count+=1
     if error is None or error>1e-4:failures.append({'owner':owner,'trace':trace['id'],'source':source,'firsthit_error':error,'firsthit':hs[0]if hs else None})
 seed=np.array(Image.open(B/'bank-exposed-rock-proposal-v1/candidate-rock-seeds.png'))>0;changed=[]
 for y,x in np.argwhere(seed):
  a,_=hits([x,y],before);b,_=hits([x,y],after(x));owner=lambda h:h[0]['owner'].split('-')[0]if h else None
  if owner(a)!=owner(b):changed.append([int(x),int(y)])
 legacy=json.loads((B/'bank-shoulder-face-routing-v1/routing.json').read_text());side=[]
 for row in legacy['rows']:
  if row['surface']!='side':continue
  hs,origin=hits(row['source'],after(row['source'][0]));actual=(origin-ray*hs[0]['distance']).tolist()if hs else None;side.append({'source':row['source'],'old_inferred_world':row['world'],'new_inferred_world':actual,'expected_owner':row['firsthit_owner'],'owner':hs[0]['owner'].split('-')[0]if hs else None,'covered':bool(hs),'old_depth_is_measurement':False})
 sidefail=[r for r in side if not r['covered']or r['owner']!=r['expected_owner']];status='PASS_SOURCE_CONSTRAINTS_CPU'if not failures and not changed and not sidefail else'HOLD';report={'status':status,'input_hashes':{str(p):hashlib.sha256(p.read_bytes()).hexdigest()for p in [O/'geometry.json',E]},'observed_crease_samples':count,'observed_crease_failures':failures,'bank_seed_samples':int(seed.sum()),'bank_seed_owner_changes':changed,'sidewall_source_samples':len(side),'sidewall_coverage_owner_failures':sidefail,'sidewall_depth_provenance':side,'limits':['Only source-facing CPU rays; no saved Boolean or oblique anatomy acceptance.','Prior26 ray-intersection depths retained as hypotheses, with coordinator-authorized correction of their evidential status.','Exact source-facing2D coverage/owner and independently supported shared crest remain required; sampled trace depth is explicitly inferred.' , 'Crease firsthits test the revised embedding against occlusion, not independent confirmation of its3D shape; saved native/oblique anatomy and contacts remain mandatory.']};(O/'combined-source-guard.json').write_text(json.dumps(report,indent=2)+'\n')
 image=Image.open(B.parent/'baseline/covered.png').convert('RGB');draw=ImageDraw.Draw(image)
 for row in d['guard_provenance']['traces']:
  points=[tuple(p)for p in row['observed_core']];draw.line(points,fill=(0,255,255),width=1)
  for p in row['reclassified_inferred_extensions']:
   x,y=p['native'];draw.ellipse((x-2,y-2,x+2,y+2),outline=(255,0,255))
 image.crop((300,330,440,465)).resize((840,810),Image.Resampling.NEAREST).save(O/'source-guard-provenance.png');print({'status':status,'creases':count,'crease_failures':len(failures),'bank_owner_changes':len(changed),'sidewall_coverage_owner_failures':len(sidefail)})
if __name__=='__main__':main()
