"""Fit a periodic C2 depth curve wholly inside continuous receiver-free bands."""
import json
from pathlib import Path
import numpy as np
from scipy.interpolate import CubicSpline
from scipy.optimize import linprog
from scipy.sparse import lil_matrix
import restart14_butterfly_canopy22_audit as reader
import restart26_butterfly07_depth_refinement as pinned
from restart26_butterfly07_depth_curve import derivative_bounds
SOURCE=reader.B/'butterfly07-v3-continuous-depth-envelope-v1/report.json'
OUT=reader.B/'butterfly07-v3-envelope-path-v1'
MIN_HEIGHT=5.;MAX_HEIGHT=220.;MAX_SPEED=7.;MAX_ACCELERATION=40.

def free_bands(forbidden):
    result=[];lo=MIN_HEIGHT
    for a,b in forbidden:
        if b<lo:continue
        if a>MAX_HEIGHT:break
        if a>lo+1e-5:result.append([lo,min(a,MAX_HEIGHT)])
        lo=max(lo,b)
    if lo<MAX_HEIGHT-1e-5:result.append([lo,MAX_HEIGHT])
    return result

def overlap(a,b):return min(a[1],b[1])-max(a[0],b[0])>1e-5

def band_paths(bands,baseline):
    candidates=[]
    for first in range(len(bands[0])):
        costs={first:(abs(np.clip(baseline[0],*bands[0][first])-baseline[0]),[first])}
        for i in range(1,len(bands)):
            nxt={}
            for j,current in enumerate(bands[i]):
                choices=[(cost+abs(np.clip(baseline[i],*current)-baseline[i]),path+[j])for previous,(cost,path)in costs.items() if overlap(bands[i-1][previous],current)]
                if choices:nxt[j]=min(choices,key=lambda item:item[0])
            costs=nxt
            if not costs:break
        endings=[(cost,path)for last,(cost,path)in costs.items()if len(path)==len(bands) and overlap(bands[-1][last],bands[0][first])]
        if endings:candidates.append(min(endings,key=lambda item:item[0]))
    return candidates

def solve_bands(bands,path,baseline,dt,max_speed=MAX_SPEED,max_acceleration=MAX_ACCELERATION):
    n=len(path);chosen=np.array([bands[i][j]for i,j in enumerate(path)]);lower=np.maximum(chosen[:,0],np.roll(chosen[:,0],1));upper=np.minimum(chosen[:,1],np.roll(chosen[:,1],1));assert np.all(lower<upper)
    objective=np.r_[np.zeros(2*n),np.ones(n),np.full(n,.05)]
    bounds=[*zip(lower,upper),*[(-max_speed,max_speed)]*n,*[(0,None)]*n,*[(0,max_acceleration)]*n]
    rows=[];limits=[]
    def inequality(terms,limit):rows.append(terms);limits.append(float(limit))
    def two_sided(terms,lo,hi):inequality(terms,hi);inequality({k:-v for k,v in terms.items()},-lo)
    equal=lil_matrix((n,4*n));rhs=np.zeros(n)
    for i in range(n):
        j=(i+1)%n;k=(i-1)%n;lo,hi=chosen[i]
        # Control points of the Hermite cubic remain within the free interval.
        two_sided({i:1,n+i:dt/3},lo,hi)
        two_sided({j:1,n+j:-dt/3},lo,hi)
        # All derivative Bezier controls are bounded; endpoint derivatives use bounds above.
        two_sided({j:3/dt,i:-3/dt,n+i:-1,n+j:-1},-max_speed,max_speed)
        acceleration={j:6/dt**2,i:-6/dt**2,n+i:-4/dt,n+j:-2/dt}
        inequality({**acceleration,3*n+i:-1},0);inequality({**{k:-v for k,v in acceleration.items()},3*n+i:-1},0)
        inequality({i:1,2*n+i:-1},baseline[i]);inequality({i:-1,2*n+i:-1},-baseline[i])
        # Equality of adjacent second derivatives gives one globally periodic C2 spline.
        equal[i,n+k]=1;equal[i,n+i]=4;equal[i,n+j]=1;equal[i,j]=-3/dt;equal[i,k]=3/dt
    matrix=lil_matrix((len(rows),4*n))
    for i,row in enumerate(rows):
        for j,value in row.items():matrix[i,j]=value
    result=linprog(objective,A_ub=matrix.tocsr(),b_ub=np.array(limits),A_eq=equal.tocsr(),b_eq=rhs,bounds=bounds,method='highs')
    if not result.success:return dict(success=False,status=result.message)
    height=result.x[:n];velocity=result.x[n:2*n];controls=np.c_[height,height+dt*velocity/3,np.roll(height,-1)-dt*np.roll(velocity,-1)/3,np.roll(height,-1)]
    violation=max(float((chosen[:,0,None]-controls).max()),float((controls-chosen[:,1,None]).max()))
    assert violation<1e-6
    times=np.arange(n+1)*dt;curve=CubicSpline(times,np.r_[height,height[0]],bc_type='periodic')
    assert np.max(abs(curve(times[:-1],1)-velocity))<1e-6
    speed,acceleration=derivative_bounds(curve);assert speed<=max_speed+1e-5 and acceleration<=max_acceleration+1e-5
    return dict(success=True,objective=float(result.fun),curve=dict(times=times.tolist(),heights=np.r_[height,height[0]].tolist()),velocity=velocity.tolist(),chosen_free_bands=chosen.tolist(),maximum_control_violation=violation,maximum_speed=speed,maximum_acceleration=acceleration,minimum_depth=float(height.min()),maximum_depth=float(height.max()),maximum_knot_deviation=float(abs(height-baseline).max()))

def main():
    assert not OUT.exists();source=json.loads(SOURCE.read_text());assert source['fit_sha256']==pinned.FIT_SHA
    map_path=reader.LIB/'scenes/croisement02.rhlos-map.json'
    assert reader.sha(map_path)==source['map_sha256'],'Receiver map changed after extraction'
    document=json.loads(map_path.read_text());sources={a['id']:a for a in document['assetSources']};expected={p['id']:sources[p['assets'][0]]['model_sha256'] for p in document['placements']}
    expected.update({a['id']:a['model_sha256'] for a in document['sceneAssets']})
    assert all(expected[a['asset']]==a['model_sha256'] for a in source['assets']),'Receiver model differs from pinned map'
    records=source['records'];n=len(records);dt=99/n;assert all(abs(r['start']-i*dt)<1e-10 and abs(r['end']-(i+1)*dt)<1e-10 for i,r in enumerate(records))
    fit=json.loads(pinned.FIT.read_text());baseline=np.array([r['fixed_path_anchor_zup'][2]for r in fit['rows']]);times=np.arange(n)*dt;baseline=np.interp(times,np.arange(100),np.r_[baseline,baseline[0]])
    bands=[free_bands(r['forbidden_height_bands'])for r in records];paths=band_paths(bands,baseline);trials=[]
    for cost,path in paths:
        result=solve_bands(bands,path,baseline,dt);result['topology_lower_bound_cost']=float(cost);trials.append(result)
    feasible=[t for t in trials if t['success']];selected=min(feasible,key=lambda t:t['objective'])if feasible else None
    output=dict(status='CONSERVATIVE_CONTINUOUS_PATH_PENDING_INDEPENDENT_ACTUAL_AUDIT'if selected else'HOLD_NO_FEASIBLE_BOUNDED_CONTINUOUS_CORRIDOR',envelope_sha256=reader.sha(SOURCE),fit_sha256=pinned.FIT_SHA,recipe_sha256=reader.sha(Path(__file__)),constraints=dict(height_range=[MIN_HEIGHT,MAX_HEIGHT],speed_z_per_phase=MAX_SPEED,acceleration_z_per_phase_squared=MAX_ACCELERATION,height_margin=source['height_margin'],objective='sum(abs(depth-baseline)) +0.05sum(abs(acceleration))'),topology_candidates=len(paths),trials=trials,selected=selected,limits=['Free-depth envelope is conservative and may exclude genuine physical gaps.','This proves the selected curve stays outside conservative static receiver bands; independent actual geometry audit follows.','No canonical changes; source quality remains held.'])
    OUT.mkdir();(OUT/'path.json').write_text(json.dumps(output,indent=2)+'\n');print(output['status'],[(t['success'],t.get('maximum_knot_deviation'))for t in trials],flush=True)

if __name__=='__main__':main()
