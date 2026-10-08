"""Bound a monotone root-back field beneath all current source-facing samples."""
import json,numpy as np
from scipy.optimize import linprog
from restart2_tree08_root_embedding_cpu import R
from restart2_tree08_junction_proof import native_depth
O=R/'tree08-monotone-field-plan-cpu-v1';O.mkdir(exist_ok=False);m=np.load(R/'tree08-root-ray-cpu-v4/candidate.npz');front=native_depth([(m['vertices'],m['faces'])]);np.savez_compressed(O/'front-depth.npz',depth=front);samples=json.loads((R/'tree08-underside-feasibility-cpu-v1/front-clearance.json').read_text());yy,xx=np.mgrid[:461,:446];x=xx+331.5;y=yy+11.5;records=[]
for name,box in [('left',[498,395,529,419]),('descending',[560,414,590,470])]:
 x0,y0,x1,y1=box;mask=np.isfinite(front)&(x>=x0)&(x<=x1)&(y>=y0)&(y<=y1);q=np.c_[x[mask]-(x0+x1)/2,y[mask]-(y0+y1)/2,np.ones(mask.sum())];fit=linprog(-q.mean(0),A_ub=q,b_ub=front[mask]-.01,bounds=[(-4,4),(-4,4),(0,2000)],method='highs');assert fit.success;covered=[]
 for s in samples:
  sx,sy=s['underside_native']
  if not x0<sx<x1 or not y0<sy<y1:continue
  threshold=float(np.array([sx-(x0+x1)/2,sy-(y0+y1)/2,1])@fit.x);gap=threshold-s['depth'];covered.append({'native_top':s['native_top'],'underside_native':s['underside_native'],'threshold_minus_back':gap,'required_multiplier':s['extension']/gap if gap>0 else None,'front_clearance':s['front_clearance']})
 records.append({'root':name,'native_box':box,'threshold_plane':fit.x.tolist(),'minimum_front_clearance':float((front[mask]-q@fit.x).min()),'covered_back_samples':sum(z['threshold_minus_back']>0 for z in covered),'sample_count':len(covered),'samples':covered})
report={'status':'FIELD FEASIBILITY ONLY; NO MESH','construction':'For native depth d, d_new = d - k*max(0,min(T(x,y)-d, box ramps)). Each active affine branch has derivative in d at least1. A conforming split at branch boundaries is required before applying this monotone map.','purpose':'One continuous field must move overlapping hidden sections coherently; independent target displacement caused crossings.','fields':records,'limitations':['The conservative affine threshold may reach only part of the required underside. Do not lift it above source first-hit constraints.','First-hit boundary points have zero hidden depth and require distinct support construction.','Source pixel, current receiver, topology and strict intersection guards remain mandatory.']};(O/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps([{k:v for k,v in r.items() if k!='samples'} for r in records],indent=2))
