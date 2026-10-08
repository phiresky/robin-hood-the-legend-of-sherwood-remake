"""CPU smooth depth proposal constrained by independently traced native root routes."""
import hashlib,json
from pathlib import Path
import numpy as np
from scipy.optimize import minimize,LinearConstraint,Bounds
R=Path(__file__).resolve().parents[3]/'level-editor/work/croisement01-refinement/restart2';out=R/'tree08-root-ray-depth-fit-v1';out.mkdir(exist_ok=False);field=np.load(R/'tree08-receiver-depth-cpu-v1/native-depth.npz');plan=json.loads((R/'tree08-topology-plan-v1/plan.json').read_text());y=np.arange(320,486,dtype=float);lower=np.zeros(len(y));samples=[]
for route in plan['structural_routes']:
 if route['name'] not in ['left basal root','descending root']:continue
 for x,py in route['source_path']:
  if py<320 or py>471:continue
  if not np.isfinite(field['wood'][py-11,x-331]):continue
  need=max(0,float(max(field['ground'][py-11,x-331],field['terrace'][py-11,x-331])-field['wood'][py-11,x-331]))+2.;lower[py-320]=max(lower[py-320],need);samples.append(dict(native=[x,py],minimum_ray_shift=need,route=route['name']))
lower[469-320]=max(lower[469-320],43.5);lower[0]=0;D=np.diff(np.eye(len(y)),axis=0);D2=np.diff(D,axis=0);Q=D2.T@D2+.000001*np.eye(len(y));initial=np.maximum(lower,np.interp(y,[320,340,369,406,428,449,469,485],[0,12,50,80,90,60,44,40]));initial[0:2]=0
constraints=[LinearConstraint(D,-2.5,1.3),LinearConstraint(np.eye(len(y))[:2],0,0)];result=minimize(lambda x:float(x@Q@x),initial,jac=lambda x:2*Q@x,method='SLSQP',bounds=Bounds(lower,np.full(len(y),120.)),constraints=constraints,options={'maxiter':1500,'ftol':1e-9});assert result.success,result.message;f=result.x;assert min(f-lower)>=-1e-6 and max(np.diff(f))<=1.300001;knots=list(zip(y[::4].tolist(),f[::4].tolist()));knots.append((float(y[-1]),float(f[-1])));interpolated=np.interp(y,*np.array(knots).T);deficit=float(max(lower-interpolated));assert deficit<.5
report=dict(status='CPU_SMOOTH_DEPTH_PROPOSAL',knots_native_y_ray_delta=knots,source_route_samples=samples,maximum_route_deficit_after_linear_resampling=deficit,positive_depth_slope=float(max(np.diff(f))),negative_depth_slope=float(min(np.diff(f))),objective=float(result.fun),core_geometry_frozen_through_native_y=320,receiver_depth_sha256=hashlib.sha256((R/'tree08-receiver-depth-cpu-v1/native-depth.npz').read_bytes()).hexdigest(),authority='Visible source route gives projected continuity. Depth remains inferred relative to unchanged current receiver. No new source RGB ownership.',limitations=['No geometry model is implied by fit success. Affine band construction, source coverage, intersections and actual/contact self-review are still required.'])
(out/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:v for k,v in report.items() if k not in ['source_route_samples','knots_native_y_ray_delta']},indent=2))
