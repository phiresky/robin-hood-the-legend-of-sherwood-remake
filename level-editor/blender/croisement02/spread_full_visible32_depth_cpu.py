"""Private inferred Tree32 branch depth, preserving all source projections.

A smooth translation along each artwork ray breaks the unsupported flat branch
arrangement. The basal region remains fixed. This grants no geometry approval.
"""
import argparse,json,hashlib
from pathlib import Path
import numpy as np
from continuous_wood_field import SIN,COS,RAY
from physical_wood_sections_cpu import projection_coverage
from physical_tree32_reviewed_domain_cpu import reviewed_source
BASE=Path(__file__).resolve().parents[3]/'level-editor/work/croisement02-refinement/wood-sweep-cpu-review'
def main():
    parser=argparse.ArgumentParser();parser.add_argument('--input',type=Path,required=True);parser.add_argument('--output',type=Path,required=True);parser.add_argument('--coefficient',type=float,default=.65);args=parser.parse_args();source=args.input;out=args.output
    if not np.isfinite(args.coefficient) or not 0<=args.coefficient<=.75:raise ValueError('Branch shear coefficient must stay within the declared .75 bound')
    if out.exists():raise FileExistsError(out)
    mesh=np.load(source);v=mesh['vertices'];f=mesh['faces'];uv=np.column_stack((v[:,0],-v[:,1]*SIN-v[:,2]*COS))
    u=np.clip((675-uv[:,1])/55,0,1);weight=u*u*(3-2*u)
    delta=args.coefficient*(uv[:,0]-1090)*weight
    new=v+delta[:,None]*RAY
    initial=np.cross(v[f[:,1]]-v[f[:,0]],v[f[:,2]]-v[f[:,0]])
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
    report['branch_depth_inference']={'source':str(source),'source_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'coefficient':args.coefficient,'maximum_ray_translation':float(abs(delta).max()),'source_projection_drift':drift,'minimum_float32_normal_fraction':float(fraction.min()),'minimum_float32_area':float(area.min()),'fixed_source_rows':'675 and below','precision_frame':'Stored local float32 about [1090,-1298,0]','scope':'New hidden branch depth only, unapproved; silhouette and known pixels unchanged; needs full crown-context review'}
    out.mkdir();np.savez_compressed(out/'preview-mesh.npz',vertices=new,faces=f);(out/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report['branch_depth_inference'],indent=2))
if __name__=='__main__':main()
