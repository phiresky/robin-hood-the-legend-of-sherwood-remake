"""Fit source-ray shears with receiver overlap and source-front visibility bounds."""
import collections,json
import numpy as np
from scipy.optimize import lsq_linear
from restart2_tree08_root_embedding_cpu import R
O=R/'tree08-bank-band-fit-cpu-v1';O.mkdir(exist_ok=False)
rows=collections.defaultdict(list)
for row in json.loads((R/'tree08-root-ray-cpu-v4/receiver-intervals.json').read_text())['samples']:
    a,b=row['wood_intervals'][-1];z=row['receiver_depth'];rows[row['native'][1]+.5].append((z+.25-b,z-.25-a,z+2.5-b))
y=np.array([320.]+sorted(rows)+[480.]);n=len(y);lower=np.full(n,-40.);upper=np.full(n,5.);target=np.zeros(n)
for i,value in enumerate(y):
    if value in rows:
        samples=rows[value];lower[i]=max(a for a,b,t in samples);upper[i]=min(b for a,b,t in samples);target[i]=np.median([t for a,b,t in samples])
lower[0],upper[0]=-1e-12,1e-12;target[-1]=target[-2];assert np.all(lower<upper)
D=np.zeros((n-2,n))
for i in range(1,n-1):
    a,b=y[i]-y[i-1],y[i+1]-y[i];D[i-1,i-1]=1/a;D[i-1,i]=-(1/a+1/b);D[i-1,i+1]=1/b
A=np.vstack([.2*np.eye(n),3*D]);b=np.r_[.2*target,np.zeros(n-2)];fit=lsq_linear(A,b,bounds=(lower,upper),tol=1e-10,max_iter=300);assert fit.success
values=fit.x;values[0]=0
report=dict(status='FIT',knots_native_y_ray_delta=np.c_[y,values].tolist(),source_front_margin=.25,receiver_embed_margin=.25,minimum_bound_slack=float(min((values[1:]-lower[1:]).min(),(upper[1:]-values[1:]).min())),cost=float(fit.cost),reason='A globally shared piecewise-affine source-ray shear preserves native XY and cannot fold continuous geometry. Actual body ray intervals include bank sides. Prior inferred depths are allowed to move; root front remains ahead of receiver and back intersects it.')
(O/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:v for k,v in report.items() if k!='knots_native_y_ray_delta'}));print('knots',n,'delta',values.min(),values.max())
