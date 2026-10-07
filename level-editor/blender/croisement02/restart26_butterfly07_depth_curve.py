"""Bound a private smooth ray-depth candidate before its full-cycle contact audit."""
import json
import numpy as np
from scipy.interpolate import CubicSpline
import restart14_butterfly07_anatomy_contacts as audit
import restart26_butterfly07_depth_refinement as sample
OUT=audit.B/'butterfly07-v3-depth-curve-v1'

def derivative_bounds(curve):
    speed=acceleration=0.
    for i,width in enumerate(np.diff(curve.x)):
        a,b,c,_=curve.c[:,i];points=[0.,width]
        if abs(a)>1e-15:
            root=-b/(3*a)
            if 0<root<width:points.append(root)
        speed=max(speed,*[abs(3*a*t*t+2*b*t+c) for t in points])
        acceleration=max(acceleration,abs(2*b),abs(6*a*width+2*b))
    return float(speed),float(acceleration)

def main():
    assert not OUT.exists()
    assert audit.reader.sha(sample.FIT)==sample.FIT_SHA
    source=sample.OUT/'corridor.json';corridor=json.loads(source.read_text())
    assert all(w['selected'] is not None for w in corridor['windows']),'No sampled corridor for at least one window'
    fit=json.loads(sample.FIT.read_text());base=np.array([r['fixed_path_anchor_zup'][2] for r in fit['rows']])
    times=np.arange(0,99.01,.25);height=np.interp(times,np.arange(100),np.r_[base,base[0]])
    for window in corridor['windows']:
        indices=np.rint(np.array(window['times'])*4).astype(int)
        height[indices]=window['selected']['height']
    curve=CubicSpline(times,height,bc_type='periodic');speed,acceleration=derivative_bounds(curve)
    definition=dict(times=times.tolist(),heights=height.tolist())
    bounded=speed<=5. and acceleration<=32.
    report=dict(status='PRIVATE_CURVE_READY_FOR_CONTACT_CHECK' if bounded else 'HOLD_INFERRED_MOTION_BOUNDS',parent_fit_sha256=sample.FIT_SHA,corridor_sha256=audit.reader.sha(source),curve=definition,maximum_depth_speed_z_per_phase=speed,maximum_depth_acceleration_z_per_phase_squared=acceleration,inferred_limits=dict(speed=5.,acceleration=32.),limits=['Periodic C2 body-depth inference; not a native observed height trajectory.','Native screen anchors, source timing, fixed geometry and source registration unchanged.','Sampled admissibility does not imply swept clearance; full-cycle check follows only when analytic motion bounds pass.','Source quality remains HOLD:344 missing,528 extra,70 bright missing pixels.'])
    OUT.mkdir();(OUT/'curve.json').write_text(json.dumps(report,indent=2)+'\n')
    if not bounded:return
    audit.OUT=OUT/'full99-contacts'
    audit.main(sample.FIT,transition_candidates={},continuous_height_curve=definition,sweep_subdivisions=8,dense_pose_steps=8,geometry_helper=sample.HELPER,geometry_helper_sha256=sample.HELPER_SHA,receiver_asset_ids=None,first_witness_only=True)

if __name__=='__main__':main()
