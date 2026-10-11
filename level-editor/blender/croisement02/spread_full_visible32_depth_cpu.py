"""Private inferred Tree32 branch depth, preserving all source projections.

Independent terminal depths, blended through the source wood domain, break the flat branch
arrangement. The basal region remains fixed. This grants no geometry approval.
"""
import argparse,json,hashlib
from pathlib import Path
import numpy as np
from continuous_wood_field import SIN,COS,RAY
from physical_wood_sections_cpu import projection_coverage
from physical_tree32_reviewed_domain_cpu import reviewed_source
from scipy.sparse import lil_matrix,coo_matrix,eye,hstack,vstack
from scipy.optimize import linprog
from scipy.sparse.linalg import spsolve
from scipy.ndimage import binary_fill_holes,distance_transform_edt,map_coordinates
BASE=Path(__file__).resolve().parents[3]/'level-editor/work/croisement02-refinement/wood-sweep-cpu-review'
def main():
    parser=argparse.ArgumentParser();parser.add_argument('--input',type=Path,required=True);parser.add_argument('--output',type=Path,required=True);parser.add_argument('--coefficient',type=float,default=.65);args=parser.parse_args();source=args.input;out=args.output
    if not np.isfinite(args.coefficient) or not 0<=args.coefficient<=.75:raise ValueError('Branch shear coefficient must stay within the declared .75 bound')
    if out.exists():raise FileExistsError(out)
    mesh=np.load(source);v=mesh['vertices'];f=mesh['faces'];uv=np.column_stack((v[:,0],-v[:,1]*SIN-v[:,2]*COS))
    report=json.loads((source.parent/'report.json').read_text());observed,path=reviewed_source(32,report['source_box']);body=binary_fill_holes(observed);origin=np.array(report['source_box'][:2]);coords=np.argwhere(body);indices=np.full(body.shape,-1,dtype=int);indices[body]=np.arange(len(coords));fixed={}
    for index,(y,x) in enumerate(coords):
        if y+origin[1]>=675:fixed[index]=0.
    anchors=[(938,568,50.),(980,557,-70.),(1055,586,45.),(1118,538,-40.),(1249,556,60.)];anchor_records=[]
    for x,y,depth in anchors:
        distance=np.linalg.norm(coords[:,::-1]+origin-np.array([x,y]),axis=1);nearest=int(distance.argmin())
        if distance[nearest]>3:raise ValueError('Branch anchor is outside the source tip')
        target=depth*args.coefficient/.65
        for i in np.flatnonzero(distance<=max(2.,distance[nearest])):fixed[int(i)]=target
        anchor_records.append(dict(source_pixel=[x,y],nearest_source_pixel=(coords[nearest,::-1]+origin).tolist(),ray_depth=target))
    matrix=lil_matrix((len(coords),len(coords)));rhs=np.zeros(len(coords))
    for index,(y,x) in enumerate(coords):
        if index in fixed:matrix[index,index]=1;rhs[index]=fixed[index];continue
        adjacent=[indices[yy,xx]for yy,xx in [(y-1,x),(y+1,x),(y,x-1),(y,x+1)]if 0<=yy<body.shape[0] and 0<=xx<body.shape[1] and body[yy,xx]]
        matrix[index,index]=len(adjacent)
        for other in adjacent:matrix[index,other]=-1
    field=np.zeros(body.shape);field[body]=spsolve(matrix.tocsr(),rhs);nearest=distance_transform_edt(~body,return_distances=False,return_indices=True);field[~body]=field[tuple(nearest[:,~body])]
    delta=map_coordinates(field,np.vstack((uv[:,1]-origin[1]-.5,uv[:,0]-origin[0]-.5)),order=1,mode='nearest');weight=(uv[:,1]<675).astype(float);delta*=weight
    initial=np.cross(v[f[:,1]]-v[f[:,0]],v[f[:,2]]-v[f[:,0]]);squared=np.sum(initial*initial,axis=1)
    px=np.clip(uv[:,0]-origin[0]-.5,0,body.shape[1]-1);py=np.clip(uv[:,1]-origin[1]-.5,0,body.shape[0]-1);ix=px.astype(int);iy=py.astype(int);fx=px-ix;fy=py-iy;nx=np.minimum(ix+1,body.shape[1]-1);ny=np.minimum(iy+1,body.shape[0]-1)
    full_index=indices[tuple(nearest)];corners=np.column_stack((full_index[iy,ix],full_index[iy,nx],full_index[ny,ix],full_index[ny,nx]));weights=np.column_stack(((1-fx)*(1-fy),fx*(1-fy),(1-fx)*fy,fx*fy))*weight[:,None]
    interpolate=coo_matrix((weights.ravel(),(np.repeat(np.arange(len(v)),4),corners.ravel())),shape=(len(v),len(coords))).tocsr()
    a=np.einsum('ij,ij->i',np.cross(RAY,v[f[:,2]]-v[f[:,0]]),initial)/squared;b=np.einsum('ij,ij->i',np.cross(v[f[:,1]]-v[f[:,0]],RAY),initial)/squared
    orient=coo_matrix((np.column_stack((-a-b,a,b)).ravel(),(np.repeat(np.arange(len(f)),3),f.ravel())),shape=(len(f),len(v))).tocsr()@interpolate;identity=eye(len(coords),format='csr');target=field[body]
    constraints=vstack([hstack([-orient,coo_matrix((len(f),len(coords)))]),hstack([identity,-identity]),hstack([-identity,-identity])],format='csr');rhs=np.r_[np.full(len(f),.65),target,-target];bounds=[(0,0)if y+origin[1]>=675 else(-100,100)for y,x in coords]+[(0,None)]*len(coords)
    importance=np.ones(len(coords));importance[[i for i in fixed if coords[i,0]+origin[1]<675]]=20
    fit=linprog(np.r_[np.zeros(len(coords)),importance],A_ub=constraints,b_ub=rhs,bounds=bounds,method='highs',options={'time_limit':180})
    if not fit.success:raise RuntimeError(fit.message)
    delta=interpolate@fit.x[:len(coords)];new=v+delta[:,None]*RAY
    q=(new-np.array([1090.,-1298.,0.])).astype(np.float32);normal=np.cross(q[f[:,1]]-q[f[:,0]],q[f[:,2]]-q[f[:,0]])
    fraction=np.sum(initial*normal,axis=1)/np.sum(initial*initial,axis=1)
    area=np.linalg.norm(normal,axis=1)/2
    if area.min()<1e-9 or fraction.min()<=0:raise ValueError(f'Precision/orientation guard: {area.min()}, {fraction.min()}')
    drift=float(np.max(abs(np.column_stack((new[:,0],-new[:,1]*SIN-new[:,2]*COS))-uv)))
    if drift>1e-8 or not np.array_equal(new[weight==0],v[weight==0]):raise ValueError('Source projection or basal region changed')
    report=json.loads((source.parent/'report.json').read_text());observed,path=reviewed_source(32,report['source_box']);coverage=projection_coverage(new,f,observed,report['source_box'][:2])
    if coverage['missing_body_centers']:raise ValueError('Source centers lost')
    report['observed_source_ground']=coverage
    sections=[];tri=new[f]
    for height in [20,40,60,80,100]:
        points=[]
        for i,j in [(0,1),(1,2),(2,0)]:
            a,b=tri[:,i],tri[:,j];hit=(a[:,2]-height)*(b[:,2]-height)<0;a,b=a[hit],b[hit];points.extend(a+(b-a)*((height-a[:,2])/(b[:,2]-a[:,2]))[:,None])
        extent=np.ptp(points,axis=0);sections.append(dict(z=height,width=float(extent[0]),depth=float(extent[1]),ratio=float(extent[1]/extent[0])))
    report['sections']=sections
    report['branch_depth_inference']={'source':str(source),'source_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'method':'Shared source-plane depth field fitted to harmonic branch-tip targets with triangle orientation constraints','solver_message':fit.message,'tip_anchors':anchor_records,'coefficient':args.coefficient,'maximum_ray_translation':float(abs(delta).max()),'source_projection_drift':drift,'minimum_float32_normal_fraction':float(fraction.min()),'minimum_float32_area':float(area.min()),'fixed_source_rows':'675 and below','precision_frame':'Stored local float32 about [1090,-1298,0]','scope':'New hidden branch depth only, unapproved; silhouette and known pixels unchanged; needs full crown-context review'}
    out.mkdir();np.savez_compressed(out/'preview-mesh.npz',vertices=new,faces=f);(out/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report['branch_depth_inference'],indent=2))
if __name__=='__main__':main()
