"""Choose free-depth bands jointly with bounded periodic motion, without greedy topology."""
import json,time
from pathlib import Path
import numpy as np
from scipy.optimize import milp,Bounds,LinearConstraint
from scipy.sparse import lil_matrix
import restart26_butterfly07_envelope_path as base


def choose_bands(bands,baseline,dt,seconds=180,display=False):
    n=len(bands);offsets=np.cumsum([0]+[len(b)for b in bands]);total=4*n+offsets[-1]
    if any(not b for b in bands):return dict(success=False,status='Empty free interval at a segment')
    c=np.r_[np.zeros(2*n),np.ones(n),np.full(n,.05),np.zeros(offsets[-1])]
    lower=np.r_[np.full(n,base.MIN_HEIGHT),np.full(n,-base.MAX_SPEED),np.zeros(2*n+offsets[-1])]
    upper=np.r_[np.full(n,base.MAX_HEIGHT),np.full(n,base.MAX_SPEED),np.full(n,np.inf),np.full(n,base.MAX_ACCELERATION),np.ones(offsets[-1])]
    integrality=np.r_[np.zeros(4*n),np.ones(offsets[-1])]
    rows=[];los=[];his=[]
    def add(terms,lo=-np.inf,hi=np.inf):rows.append(terms);los.append(lo);his.append(hi)
    span=base.MAX_HEIGHT-base.MIN_HEIGHT+2*base.MAX_SPEED*dt
    for i in range(n):
        j=(i+1)%n;k=(i-1)%n
        controls=[{i:1},{i:1,n+i:dt/3},{j:1,n+j:-dt/3},{j:1}]
        add({4*n+v:1 for v in range(offsets[i],offsets[i+1])},1,1)
        for b,(lo,hi)in enumerate(bands[i]):
            binary=4*n+offsets[i]+b
            for control in controls:
                add({**control,binary:-span},lo-span,np.inf)
                add({**control,binary:span},-np.inf,hi+span)
        add({j:3/dt,i:-3/dt,n+i:-1,n+j:-1},-base.MAX_SPEED,base.MAX_SPEED)
        acceleration={j:6/dt**2,i:-6/dt**2,n+i:-4/dt,n+j:-2/dt}
        add({**acceleration,3*n+i:-1},hi=0)
        add({**{k:-v for k,v in acceleration.items()},3*n+i:-1},hi=0)
        add({i:1,2*n+i:-1},hi=baseline[i]);add({i:-1,2*n+i:-1},hi=-baseline[i])
        add({n+k:1,n+i:4,n+j:1,j:-3/dt,k:3/dt},0,0)
    matrix=lil_matrix((len(rows),total))
    for i,row in enumerate(rows):
        for j,value in row.items():matrix[i,j]=value
    start=time.monotonic()
    result=milp(c,integrality=integrality,bounds=Bounds(lower,upper),constraints=LinearConstraint(matrix.tocsr(),los,his),options=dict(time_limit=seconds,mip_rel_gap=.02,disp=display,threads=2))
    output=dict(success=result.x is not None,status=result.message,elapsed_seconds=time.monotonic()-start,solver_status=int(result.status),variable_count=int(total),binary_count=int(offsets[-1]))
    if result.x is not None:
        output['height']=result.x[:n].tolist();output['velocity']=result.x[n:2*n].tolist()
        output['path']=[int(np.argmax(result.x[4*n+offsets[i]:4*n+offsets[i+1]]))for i in range(n)]
        output['objective']=float(result.fun);output['gap']=float(result.mip_gap)
    return output


def main():
    import argparse,shutil
    parser=argparse.ArgumentParser();parser.add_argument('--candidate',type=Path,required=True);parser.add_argument('--envelope',type=Path,required=True);parser.add_argument('--output',type=Path,required=True);args=parser.parse_args();assert not args.output.exists();args.output.mkdir()
    from scipy.interpolate import CubicSpline
    source=args.candidate/'fit.json';fit=json.loads(source.read_text());env=json.loads(args.envelope.read_text());assert base.reader.sha(source)==env['fit_sha256'];assert base.reader.sha(base.reader.LIB/'scenes/croisement02.rhlos-map.json')==env['map_sha256']
    inputs=json.loads((args.candidate/'inputs.json').read_text());old=inputs['continuous_height_curve'];prior=CubicSpline(old['times'],old['heights'],bc_type='periodic');n=len(env['records']);dt=99/n;times=np.arange(n+1)*dt;baseline=prior(times[:-1]);bands=[base.free_bands(r['forbidden_height_bands'])for r in env['records']];result=choose_bands(bands,baseline,dt,display=True)
    report={'status':'HOLD_NO_FEASIBLE_PRIMAL','selection':result,'envelope_sha256':base.reader.sha(args.envelope),'parent_fit_sha256':base.reader.sha(source),'recipe_sha256':base.reader.sha(Path(__file__)),'scope':'Only receiver primitives explicitly included in the pinned envelope; full map and moving receiver validation still required.'}
    if result['success']:
        height=np.array(result['height']);velocity=np.array(result['velocity']);chosen=np.array([bands[i][j]for i,j in enumerate(result['path'])]);curve=CubicSpline(times,np.r_[height,height[0]],bc_type='periodic');controls=np.c_[height,height+dt*velocity/3,np.roll(height,-1)-dt*np.roll(velocity,-1)/3,np.roll(height,-1)];violation=max(float((chosen[:,0,None]-controls).max()),float((controls-chosen[:,1,None]).max()));velocity_error=float(abs(curve(times[:-1],1)-velocity).max());speed,acceleration=base.derivative_bounds(curve);assert violation<1e-6 and velocity_error<1e-6 and speed<=base.MAX_SPEED+1e-5 and acceleration<=base.MAX_ACCELERATION+1e-5,(violation,velocity_error,speed,acceleration)
        report.update(status='AUDITED_MILP_PRIMAL_CUBIC_REQUIRES_ACTUAL_CONTACT_REVIEW',control_violation=violation,velocity_readback_error=velocity_error,max_speed=speed,max_acceleration=acceleration,max_knot_deviation=float(abs(height-baseline).max()))
        helper=args.output/'geometry.py';shutil.copyfile(args.candidate/'geometry.py',helper)
        for row in fit['rows']:
            x,y=row['source']['alpha_centroid_display'];z=float(curve(row['phase']));row['fixed_path_anchor_zup']=[x,-(y+base.reader.COS*z)/base.reader.SIN,z]
        target=args.output/'fit.json';target.write_text(json.dumps(fit,indent=2)+'\n');inputs.update(status='PRIVATE_AUDITED_MILP_DEPTH_CANDIDATE',continuous_height_curve={'times':times.tolist(),'heights':np.r_[height,height[0]].tolist()},joined_fit_sha256=base.reader.sha(target),geometry_helper_sha256=base.reader.sha(helper),envelope_sha256=base.reader.sha(args.envelope));(args.output/'inputs.json').write_text(json.dumps(inputs,indent=2)+'\n')
    (args.output/'solver.json').write_text(json.dumps(report,indent=2)+'\n');print(report['status'])
if __name__=='__main__':main()
