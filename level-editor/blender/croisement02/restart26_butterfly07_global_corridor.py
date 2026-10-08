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
    result=milp(c,integrality=integrality,bounds=Bounds(lower,upper),constraints=LinearConstraint(matrix.tocsr(),los,his),options=dict(time_limit=seconds,mip_rel_gap=.02,disp=display))
    output=dict(success=result.x is not None,status=result.message,elapsed_seconds=time.monotonic()-start,solver_status=int(result.status),variable_count=int(total),binary_count=int(offsets[-1]))
    if result.x is not None:
        output['path']=[int(np.argmax(result.x[4*n+offsets[i]:4*n+offsets[i+1]]))for i in range(n)]
        output['objective']=float(result.fun);output['gap']=float(result.mip_gap)
    return output


def main():
    source=base.reader.B/'butterfly07-v3-piece-depth-envelope-v2/report.json'
    out=base.reader.B/'butterfly07-v3-global-corridor-v1';assert not out.exists();out.mkdir()
    envelope=json.loads(source.read_text());assert envelope['fit_sha256']==base.pinned.FIT_SHA
    assert base.reader.sha(base.reader.LIB/'scenes/croisement02.rhlos-map.json')==envelope['map_sha256']
    rows=json.loads(base.pinned.FIT.read_text())['rows'];z=np.array([r['fixed_path_anchor_zup'][2]for r in rows]);n=len(envelope['records']);dt=99/n
    baseline=np.interp(np.arange(n)*dt,np.arange(100),np.r_[z,z[0]])
    bands=[base.free_bands(r['forbidden_height_bands'])for r in envelope['records']]
    selected=choose_bands(bands,baseline,dt,display=True)
    continuous=base.solve_bands(bands,selected['path'],baseline,dt)if selected['success']else None
    result=dict(status='BOUNDED_CORRIDOR_PENDING_ACTUAL_AUDIT'if continuous and continuous['success']else'HOLD_NO_BOUNDED_CORRIDOR_FOUND',envelope_sha256=base.reader.sha(source),fit_sha256=base.pinned.FIT_SHA,recipe_sha256=base.reader.sha(Path(__file__)),selection=selected,continuous=continuous,limits=['Source fit remains held; exact anatomy unchanged.','A time-limit failure is not an infeasibility proof.','Conservative receiver envelopes can exclude genuine gaps.','Independent actual geometry audit required for any solution.'])
    (out/'result.json').write_text(json.dumps(result,indent=2)+'\n');print(result['status'],flush=True)

if __name__=='__main__':main()
