"""Match actual retained loops to CPU field cuts without angle sorting or saves."""
import argparse,hashlib,json
from pathlib import Path
import numpy as np
from scipy.optimize import linear_sum_assignment
from scipy.spatial import cKDTree
from continuous_wood_field import collar,collar_quality
ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/croisement02-refinement/wood-sweep-cpu-review'

def loop_data(points,normals):
    p=np.asarray(points,float);n=np.asarray(normals,float)
    area=np.sum(p[:,0]*np.roll(p[:,1],-1)-np.roll(p[:,0],-1)*p[:,1])
    if abs(area)<1e-8:raise ValueError('Collapsed planar boundary')
    if area<0:p=p[::-1];n=n[::-1]
    lengths=np.linalg.norm(np.roll(p,-1,axis=0)-p,axis=1)
    if lengths.min()<1e-10:raise ValueError('Repeated boundary vertex')
    t=np.r_[0,np.cumsum(lengths)[:-1]]/lengths.sum()
    return p,n,t

def sample(values,t,query):
    closed=np.vstack([values,values[0]]);abscissa=np.r_[t,1.]
    return np.column_stack([np.interp(np.mod(query,1),abscissa,closed[:,i]) for i in range(values.shape[1])])

def fit(lower,upper,*,include_geometry=False,tangent_mode="up-projection",forced_phase=None,tangent_smoothing=0.):
    lo,ln,lt=loop_data(lower['positions'],lower['normals']);hi,hn,ht=loop_data(upper['positions'],upper['normals'])
    queries=np.arange(128)/128;high=sample(hi,ht,queries);high-=high.mean(axis=0);high/=np.sqrt(np.mean(high[:,:2]**2))
    costs=[]
    for shift in queries:
        low=sample(lo,lt,queries+shift);low-=low.mean(axis=0);low/=np.sqrt(np.mean(low[:,:2]**2));costs.append(np.mean((low[:,:2]-high[:,:2])**2))
    phase=float(queries[int(np.argmin(costs))]) if forced_phase is None else float(forced_phase)%1.;t=np.unique(np.r_[ht,np.mod(lt-phase,1.)])
    lower_points=sample(lo,lt,t+phase);upper_points=sample(hi,ht,t)
    lower_normals=sample(ln,lt,t+phase);upper_normals=sample(hn,ht,t)
    normal_regularization=[]
    if tangent_smoothing:
        from scipy.ndimage import gaussian_filter1d
        smoothed=[]
        for points,normals,abscissa,queries_now,original in [(lo,ln,lt,t+phase,lower_normals),(hi,hn,ht,t,upper_normals)]:
            uniform=np.arange(1024)/1024
            perimeter=np.linalg.norm(np.roll(points,-1,axis=0)-points,axis=1).sum()
            values=gaussian_filter1d(sample(normals,abscissa,uniform),tangent_smoothing*1024/perimeter,axis=0,mode='wrap')
            revised=sample(values,uniform,queries_now)
            revised/=np.linalg.norm(revised,axis=1)[:,None]
            unit=original/np.linalg.norm(original,axis=1)[:,None]
            angles=np.degrees(np.arccos(np.clip(np.sum(unit*revised,axis=1),-1,1)))
            normal_regularization.append(dict(radius=tangent_smoothing,maximum_degrees=float(angles.max()),p95_degrees=float(np.quantile(angles,.95))))
            smoothed.append(revised)
        lower_normals,upper_normals=smoothed
    tangents=[];height=float(hi[0,2]-lo[0,2])
    if height<=0:raise ValueError('Collar height is not positive')
    for normals,points in [(lower_normals,lower_points),(upper_normals,upper_points)]:
        normals/=np.linalg.norm(normals,axis=1)[:,None]
        if tangent_mode=='boundary-cross':
            forward=np.roll(points,-1,axis=0)-points;backward=points-np.roll(points,1,axis=0)
            around=forward/np.linalg.norm(forward,axis=1)[:,None]+backward/np.linalg.norm(backward,axis=1)[:,None]
            around/=np.linalg.norm(around,axis=1)[:,None]
            tangent=np.cross(normals,around);tangent=np.where(tangent[:,2,None]<0,-tangent,tangent)
        elif tangent_mode=='up-projection':
            tangent=np.array([0.,0.,1.])-normals*normals[:,2,None]
        else:raise ValueError('Unknown tangent mode')
        length=np.linalg.norm(tangent,axis=1)
        if np.any(length<1e-6):raise ValueError('Horizontal cap normal cannot define upward collar tangent')
        tangents.append(tangent/length[:,None]*height)
    attempts=[]
    for tangent_scale in [1.,.5,.25,.125,.0625,.03125]:
        endpoint_samples=np.array([0,1e-5,1e-4,.0005,.001,.002,.005,.01])
        parameters=np.unique(np.r_[endpoint_samples,np.linspace(0,1,49),1-endpoint_samples])
        rows=collar(lower_points,upper_points,*(v*tangent_scale for v in tangents),parameters=parameters);quality=collar_quality(rows);attempts.append(dict(tangent_scale=tangent_scale,**quality))
        if quality['collapsed_quads']==quality['reversed_quads']==quality['undefined_reference_normals']==0:break
    original_errors=[float(cKDTree(points).query(original)[0].max()) for points,original in [(lower_points,lo),(upper_points,hi)]]
    if max(original_errors)>1e-7:raise ValueError('Existing boundary vertex lost during edge subdivision')
    result=dict(lower_height=float(lo[0,2]),upper_height=float(hi[0,2]),lower_vertices=len(lo),upper_vertices=len(hi),common_subdivision_vertices=len(t),tangent_mode=tangent_mode,normal_regularization=normal_regularization,lower_phase=phase,normalized_shape_cost=float(min(costs)),original_boundary_vertex_error=original_errors,quality=quality,tangent_scale=tangent_scale,tangent_scale_attempts=attempts,eligible_for_bounded_integration=quality['collapsed_quads']==quality['reversed_quads']==quality['undefined_reference_normals']==0,limits=['CPU correspondence only; no saved mesh or source rasterization approval.','Existing boundary edges would be subdivided without moving original points.','Actual collar source coverage, physical contact and solid/actual appearance remain required.'])
    if include_geometry:
        result['geometry']={'rows':rows.tolist(),'parameters':parameters.tolist(),'lower_original':lo.tolist(),'upper_original':hi.tolist(),'lower_correspondence':np.mod(t+phase,1.).tolist(),'upper_correspondence':t.tolist()}
    return result

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--field-cuts',type=Path,default=OUT/'field-collar-cuts-v2.json');parser.add_argument('--retained',type=Path,default=OUT/'retained-collars-v1.json');parser.add_argument('--output',type=Path,default=OUT/'collar-fitting-v1.json');args=parser.parse_args()
    field=json.loads(args.field_cuts.read_text());retained=json.loads(args.retained.read_text());reports=[]
    for old in retained['records']:
        fresh=next(r for r in field['records'] if r['tree']==old['tree'])
        for upper_cut in old['cuts']:
            uppers=[dict(positions=[upper_cut['vertices'][str(i)]['position'] for i in loop],normals=[upper_cut['vertices'][str(i)]['geometric_normal'] for i in loop]) for loop in upper_cut['ordered_loops']]
            for lower_cut in fresh['cuts']:
                lowers=lower_cut['loops'];height=upper_cut['height']-lower_cut['height']
                if not 5<=height<=30 or len(lowers)!=len(uppers):continue
                costs=np.array([[np.linalg.norm(np.mean(lo['positions'],axis=0)[:2]-np.mean(hi['positions'],axis=0)[:2]) for hi in uppers] for lo in lowers]);li,ui=linear_sum_assignment(costs)
                for l,u in zip(li,ui):
                    try:record=fit(lowers[l],uppers[u])
                    except ValueError as error:record=dict(eligible_for_bounded_integration=False,error=str(error))
                    reports.append(dict(tree=old['tree'],lower_height=lower_cut['height'],upper_height=upper_cut['height'],lower_loop=int(l),upper_loop=int(u),**{k:v for k,v in record.items() if k not in ['lower_height','upper_height']}))
    sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
    result=dict(status='CPU loop matching only; no model or canonical changes',inputs=[dict(path=str(p.resolve()),sha256=sha(p)) for p in [args.field_cuts,args.retained]],records=reports)
    args.output.write_text(json.dumps(result,indent=2)+'\n')
    for r in reports:print(r['tree'],r['lower_height'],r['upper_height'],r['lower_loop'],r['eligible_for_bounded_integration'],r.get('quality',r.get('error')))
if __name__=='__main__':main()
