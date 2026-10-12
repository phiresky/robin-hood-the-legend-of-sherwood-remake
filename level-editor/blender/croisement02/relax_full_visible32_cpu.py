"""Fair the inferred Tree32 trunk depth while retaining its source projection."""
import json,hashlib
from pathlib import Path
import numpy as np
from scipy.interpolate import CubicSpline
from scipy.ndimage import gaussian_filter1d
from continuous_wood_field import SIN,COS,RAY
from scipy.optimize import linprog
from scipy.sparse import coo_matrix,eye,hstack,vstack
from physical_wood_sections_cpu import projection_coverage
from physical_tree32_reviewed_domain_cpu import reviewed_source
BASE=Path(__file__).resolve().parents[3]/'level-editor/work/active/croisement02/wood-sweep-cpu-review'
def step(x):
    t=np.clip(x,0,1);return t*t*(3-2*t)
def main():
    source=BASE/'physical-full-visible32-clean-oct11-v1/preview-mesh.npz';out=BASE/'physical-full-visible32-fair-oct11-v1'
    if out.exists():raise FileExistsError(out)
    m=np.load(source);v=m['vertices'];f=m['faces'];tri=v[f];zs=np.arange(15.,136.);depth=[];centers=[]
    for z in zs:
        points=[]
        for i,j in [(0,1),(1,2),(2,0)]:
            a,b=tri[:,i],tri[:,j];hit=(a[:,2]-z)*(b[:,2]-z)<0;a,b=a[hit],b[hit];points.extend(a+(b-a)*((z-a[:,2])/(b[:,2]-a[:,2]))[:,None])
        points=np.asarray(points);lo,hi=points[:,1].min(),points[:,1].max();depth.append((hi-lo)/2);centers.append((hi+lo)/2)
    scale=CubicSpline(zs,gaussian_filter1d(np.clip(25/np.asarray(depth),.75,1.25),3),bc_type='natural');z=v[:,2];weight=step((z-15)/25)*step((135-z)/30)
    k=1+weight*(scale(np.clip(z,zs[0],zs[-1]))-1);dy=(v[:,1]-float(np.median(centers)))*(k-1)
    initial=np.cross(tri[:,1]-tri[:,0],tri[:,2]-tri[:,0]);squared=np.sum(initial*initial,axis=1);count=len(v)
    a=np.einsum('ij,ij->i',np.cross(RAY,tri[:,2]-tri[:,0]),initial)/squared
    b=np.einsum('ij,ij->i',np.cross(tri[:,1]-tri[:,0],RAY),initial)/squared
    coefficients=np.column_stack((-a-b,a,b));orientation=coo_matrix((coefficients.ravel(),(np.repeat(np.arange(len(f)),3),f.ravel())),shape=(len(f),count)).tocsr();identity=eye(count,format='csr')
    constraints=vstack([hstack([-orientation,coo_matrix((len(f),count))]),hstack([identity,-identity]),hstack([-identity,-identity])],format='csr');target=-dy/COS;rhs=np.r_[np.full(len(f),.65),target,-target]
    bounds=[(-float(10*w),float(10*w))for w in weight]+[(0,None)]*count
    result=linprog(np.r_[np.zeros(count),np.ones(count)],A_ub=constraints,b_ub=rhs,bounds=bounds,method='highs',options={'time_limit':180})
    if not result.success:raise RuntimeError(result.message)
    new=v+result.x[:count,None]*RAY
    q=(new-np.array([1090.,-1298.,0.])).astype(np.float32)[f];normal=np.cross(q[:,1]-q[:,0],q[:,2]-q[:,0]);fraction=np.sum(normal*initial,axis=1)/np.sum(initial*initial,axis=1);area=np.linalg.norm(normal,axis=1)/2
    if fraction.min()<=0 or area.min()<1e-9:raise ValueError(f'Orientation or precision failure: {fraction.min()}, {area.min()}')
    projection=lambda p:np.column_stack((p[:,0],-p[:,1]*SIN-p[:,2]*COS));drift=float(abs(projection(new)-projection(v)).max())
    if drift>1e-8 or not np.array_equal(new[(z<=15)|(z>=135)],v[(z<=15)|(z>=135)]):raise ValueError('Source or fixed root/branch region changed')
    report=json.loads((source.parent/'report.json').read_text());observed,path=reviewed_source(32,report['source_box']);coverage=projection_coverage(new,f,observed,report['source_box'][:2])
    if coverage['missing_body_centers']:raise ValueError('Source centers lost')
    report['observed_source_ground']=coverage;report['trunk_fairing']={'input':str(source),'input_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'maximum_displacement':float(np.linalg.norm(new-v,axis=1).max()),'source_projection_drift':drift,'minimum_normal_fraction':float(fraction.min()),'minimum_area':float(area.min()),'target_half_depth':25,'solver_message':result.message,'fixed_z_regions':[15,135],'precision_frame':'Stored local float32 about [1090,-1298,0]','scope':'Private anatomy correction; retained source projection and fixed basal/branch vertices; saved context review still required'}
    out.mkdir();np.savez_compressed(out/'preview-mesh.npz',vertices=new,faces=f);(out/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report['trunk_fairing'],indent=2))
if __name__=='__main__':main()
