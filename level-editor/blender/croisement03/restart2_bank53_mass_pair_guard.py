"""Focused private53-versus52 source trace depth diagnosis, not full bank acceptance."""
import argparse,hashlib,json,math
from pathlib import Path
import numpy as np
S=math.sin(math.radians(35));C=math.cos(math.radians(35))
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def heights(mesh,q):
 v=np.array(mesh['vertices']);projected=np.column_stack((v[:,0],-v[:,1]*S-v[:,2]*C));nearest=np.full(len(q),-np.inf)
 for face in mesh['faces']:
  t=projected[face];a=t[1]-t[0];b=t[2]-t[0];det=a[0]*b[1]-a[1]*b[0]
  if abs(det)<1e-10:continue
  k=q-t[0];u=(k[:,0]*b[1]-k[:,1]*b[0])/det;w=(a[0]*k[:,1]-a[1]*k[:,0])/det;inside=(u>=-1e-8)&(w>=-1e-8)&(u+w<=1+1e-8);z=v[face[0],2]+u*(v[face[1],2]-v[face[0],2])+w*(v[face[2],2]-v[face[0],2]);nearest[inside]=np.maximum(nearest[inside],z[inside])
 return nearest

def main():
 p=argparse.ArgumentParser();p.add_argument('--geometry53',type=Path,required=True);p.add_argument('--geometry52',type=Path,required=True);p.add_argument('--old-guard',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args();m53=json.loads(a.geometry53.read_text());m52=json.loads(a.geometry52.read_text())['52'];old=json.loads(a.old_guard.read_text());rows=[r for r in old['observed_crease_failures']if r['owner']=='53' and r['firsthit']['owner']=='52-left'];q=np.array([r['source']for r in rows]);h53,h52=heights(m53,q),heights(m52,q);remaining=h53<=h52+1e-5
 sources=[];expected=[];names=[];v=np.array(m52['vertices'])
 for t in m52['traces']:
  points=np.array(t.get('source_path_world',[v[i]for i in t['vertices']]))
  for x,y in zip(points,points[1:]):
   for frac in np.linspace(0,1,31):
    point=x*(1-frac)+y*frac;sources.append([point[0],-point[1]*S-point[2]*C]);expected.append(point[2]);names.append(t['id'])
 occ=heights(m53,np.array(sources))>np.array(expected)+1e-5;a.output.mkdir(exist_ok=False);report={'status':'PASS_FOCUSED_PAIR_TRACES'if not remaining.any()and not occ.any()else'HOLD_PAIR_TRACE_CONFLICTS','pins':{str(p):sha(p)for p in [a.geometry53,a.geometry52,a.old_guard]},'old_failed53_samples':len(q),'remaining53_hidden':int(remaining.sum()),'remaining53':[{**row,'new_authored_height_margin':float((h53[i]-h52[i])*C)}for i,row in enumerate(rows)if remaining[i]],'sampled52_trace_points':len(sources),'new53_occludes52':int(occ.sum()),'occluded52':[[names[i],sources[i]]for i in np.flatnonzero(occ)],'limits':['This diagnoses prior53 failures and sampled52 traces; full protectedowners, neighbors, sidewalls and selfintersection guards remain required.']};(a.output/'receipt.json').write_text(json.dumps(report,indent=2)+'\n');print({k:report[k]for k in ['status','old_failed53_samples','remaining53_hidden','sampled52_trace_points','new53_occludes52']})
if __name__=='__main__':main()
