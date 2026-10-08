"""Fit local monotone depth thresholds against exact receiver-contact deficits."""
import json,sys,numpy as np
from scipy.optimize import linprog
from restart2_tree08_root_embedding_cpu import R
O=R/('tree08-contact-field-fit-cpu-v2' if '--minimum' in sys.argv else 'tree08-contact-field-fit-cpu-v1');O.mkdir(exist_ok=False);front=np.load(R/'tree08-monotone-field-plan-cpu-v1/front-depth.npz')['depth'];rows=json.loads((R/'tree08-monotone-back-cpu-v2/receiver-intervals.json').read_text())['samples'];yy,xx=np.mgrid[:461,:446];x=xx+331.5;y=yy+11.5;records=[]
for name,box in [('left-contact',[502,401,514,411]),('descending-contact',[576,430,589,463])]:
 x0,y0,x1,y1=box;mx,my=(x0+x1)/2,(y0+y1)/2;mask=np.isfinite(front)&(x>=x0)&(x<=x1)&(y>=y0)&(y<=y1);q=np.c_[x[mask]-mx,y[mask]-my,np.ones(mask.sum())];a=[q];b=[front[mask]-.01];samples=[]
 for row in rows:
  sx,sy=np.array(row['native'])+.5
  if row['status']!='NO_RECEIVER_INTERSECTION' or not x0<sx<x1 or not y0<sy<y1:continue
  amount=row['front_interval_back_clearance']+.25;assert amount<=40 and amount<=6*min(sx-x0,x1-sx,sy-y0,y1-sy);threshold=row['wood_intervals'][-1][0]+amount/8;a.append(-np.array([[sx-mx,sy-my,1]]));b.append(np.array([-threshold]));samples.append({'native':row['native'],'requested_extension':amount,'minimum_threshold':threshold})
 matrix=np.concatenate(a);rhs=np.concatenate(b)
 if '--minimum' in sys.argv:
  matrix=np.vstack([np.c_[matrix,np.zeros((len(matrix),2))],[1,0,0,-1,0],[-1,0,0,-1,0],[0,1,0,0,-1],[0,-1,0,0,-1]]);rhs=np.r_[rhs,[0,0,0,0]];fit=linprog([0,0,1,4,4],A_ub=matrix,b_ub=rhs,bounds=[(-4,4),(-4,4),(0,2000),(0,4),(0,4)],method='highs')
 else:fit=linprog(-q.mean(0),A_ub=matrix,b_ub=rhs,bounds=[(-4,4),(-4,4),(0,2000)],method='highs')
 entry={'root':name,'native_box':box,'status':'FIT' if fit.success else 'INFEASIBLE','samples':samples,'message':fit.message}
 if fit.success:entry['threshold_plane']=fit.x[:3].tolist();entry['minimum_front_clearance']=float((front[mask]-q@fit.x[:3]).min())
 records.append(entry)
report={'status':'CPU FIELD FIT ONLY','base_candidate':'tree08-monotone-back-cpu-v2','fields':records,'scope':'Local affine threshold is bounded below every existing native first hit; requested deformation reaches actual receiver surfaces inside closed wood ray intervals.','guards':'Conforming plane splits, saved-float source/depth/topology and strict intersections still required.'};(O/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps([{k:v for k,v in x.items() if k!='samples'} for x in records],indent=2))
