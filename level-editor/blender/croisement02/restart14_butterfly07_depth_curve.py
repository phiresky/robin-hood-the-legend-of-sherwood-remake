"""Fit a periodic C2 inferred depth curve through exact sampled free depths."""
import json
import numpy as np
from scipy.interpolate import CubicSpline
import restart14_butterfly07_anatomy_contacts as audit
B=audit.B;OUT=B/'butterfly07-depth-curve-v1'

def main():
    assert not OUT.exists();source=B/'footprint-path-proposal-v2/proposal.json';baseline=next(r for r in json.loads(source.read_text())['rows']if r['sequence']==14);h=np.array(baseline['world_zup_knots'])[:,2];corridorp=B/'butterfly07-depth-corridor-v1/corridor-free-boundaries.json';corridor=json.loads(corridorp.read_text());times=np.arange(0,99.01,.25);height=np.interp(times,np.arange(100),np.r_[h,h[0]]);delta=np.zeros(len(times))
    for window in corridor['windows']:
        assert window['selected'] is not None
        t=np.array(window['times']);d=np.array(window['selected']['delta']);knots=np.r_[t[0]-2,t,t[-1]+2];values=np.r_[0,d,0];active=(times>=knots[0])&(times<=knots[-1]);delta[active]+=np.interp(times[active],knots,values)
    height+=delta;curve=CubicSpline(times,height,bc_type='periodic');speed=0.;curvature=0.
    for i,width in enumerate(np.diff(times)):
        a,b,c,_=curve.c[:,i];at=[0.,width]
        if abs(a)>1e-15:
            root=-b/(3*a)
            if 0<root<width:at.append(root)
        speed=max(speed,*[abs(3*a*x*x+2*b*x+c)for x in at]);curvature=max(curvature,abs(2*b),abs(6*a*width+2*b))
    definition={'times':times.tolist(),'heights':height.tolist()};report={'status':'PRIVATE_INFERRED_PERIODIC_DEPTH_REQUIRES_FULL_SCENE_AUDIT','source_sha256':audit.reader.sha(source),'corridor_sha256':audit.reader.sha(corridorp),'curve':definition,'max_depth_speed_per_phase':float(speed),'max_depth_acceleration_per_phase_squared':float(curvature),'maximum_delta_at_knots':float(abs(delta).max()),'screen_and_metadata':'Exact99native screen anchors and timing; native elevation100/order unchanged. Curve depth independently inferred.','limits':['Spline may cross contact regions between accepted samples; actual swept geometry checked next.','Only8own anatomical poses built, missing sourcepixels retained. Complete99source footprint check separate.']};OUT.mkdir();(OUT/'curve.json').write_text(json.dumps(report,indent=2)+'\n');print('bounds',speed,curvature,flush=True);audit.OUT=B/'butterfly07-depth-curve-motion-v1';audit.main(B/'butterfly07-body-axis-selected-v1/fit.json',continuous_height_curve=definition)

if __name__=='__main__':main()
