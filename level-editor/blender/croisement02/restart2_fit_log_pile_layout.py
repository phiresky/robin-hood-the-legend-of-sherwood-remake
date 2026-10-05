"""Test native cap placement against a complete six-course triangular stack."""
import json,math
import numpy as np
from scipy.optimize import least_squares
from catalog import OUT


def main():
    old=json.loads((OUT/'log-trap-state-candidate-v14/manifest.json').read_text());rows=np.array(old['surveys']['initial']);observed=rows[:,:4].reshape(7,2,2)+[505,453];S,C=math.sin(math.radians(35)),math.cos(math.radians(35));bank=43.9491081237793
    prior=json.loads((OUT/'restart2-state/log-triangular-pile-v1/fit.json').read_text());slots=np.array([[0,0],[1,0],[2,0],[3,0],[4,0],[4,1],[5,0]]);u=np.array(prior['axis']);v=np.array(prior['cross_axis_away_from_camera']);theta=math.atan2(u[1],u[0]);front=[r for r in prior['records']if r['column']==0];cent=np.array([(np.array(r['start'])+r['end'])/2 for r in front]);length=np.array([np.linalg.norm(np.array(r['end'])-r['start'])for r in front])
    def geometry(p):
        theta,r,s0=p[:3];u=np.array([math.cos(theta),math.sin(theta),0]);v=np.array([-u[1],u[0],0]);centers=p[3:10,None]*u+(s0+(slots[:,0]+2*slots[:,1])*r)[:,None]*v;centers[:,2]=bank+r+slots[:,0]*math.sqrt(3)*r;ends=np.stack([centers-p[10:,None]*u/2,centers+p[10:,None]*u/2],axis=1);projected=np.stack([ends[:,:,0],-S*ends[:,:,1]-C*ends[:,:,2]],axis=2);return centers,ends,projected,u,v
    start=np.r_[theta,6.5,cent[0]@v,cent@u,length];lo=np.r_[-.9,5,-np.inf,np.full(7,-np.inf),np.full(7,60)];hi=np.r_[-.35,9,np.inf,np.full(7,np.inf),np.full(7,170)]
    fit=least_squares(lambda p:np.r_[(geometry(p)[2]-observed).ravel(),(p[1]-rows[:,4].mean())],start,bounds=(lo,hi),max_nfev=500,ftol=1e-12,xtol=1e-12,gtol=1e-12)
    centers,ends,projected,u,v=geometry(fit.x);records=[]
    for layer in range(6):
        for column in range(6-layer):
            matches=np.flatnonzero(np.all(slots==[layer,column],axis=1));observed_index=int(matches[0])if len(matches)else None;reference=observed_index if observed_index is not None else int(np.flatnonzero(slots[:,0]==layer)[0]);c=fit.x[3+reference]*u+(fit.x[2]+(layer+2*column)*fit.x[1])*v;c[2]=bank+fit.x[1]+layer*math.sqrt(3)*fit.x[1];records.append(dict(layer=layer,column=column,observed_index=observed_index,role='observed source-facing log'if observed_index is not None else'inferred supporting course',start=(c-fit.x[10+reference]*u/2).tolist(),end=(c+fit.x[10+reference]*u/2).tolist(),radius=float(fit.x[1])))
    dest=OUT/'restart2-state/log-triangular-pile-v4';dest.mkdir(exist_ok=False);report={**prior,'status':'Private source-layout hypothesis; seven visible logs distributed across six complete supporting courses','records':records,'radius':float(fit.x[1]),'layer_counts':[6,5,4,3,2,1],'observed_slots':slots.tolist(),'axis':u.tolist(),'cross_axis_away_from_camera':v.tolist(),'projected_endpoints':projected.tolist(),'endpoint_error_pixels':np.linalg.norm(projected-observed,axis=2).tolist(),'maximum_endpoint_error_pixels':float(np.linalg.norm(projected-observed,axis=2).max()),'limitations':['Seven native-visible logs do not necessarily imply seven vertical courses. This hypothesis places two upper logs in one course.','Fourteen hidden supporting logs are inferred.','Native silhouette, longitudinal support and bank contact must be audited before rendering.']};(dest/'fit.json').write_text(json.dumps(report,indent=2)+'\n');print(report['maximum_endpoint_error_pixels'],report['radius'])
if __name__=='__main__':main()
