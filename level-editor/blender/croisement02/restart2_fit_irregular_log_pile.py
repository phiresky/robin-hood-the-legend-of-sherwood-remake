"""Fit supported irregular triangular courses before scheduling any solid renders."""
import json,math,hashlib
import numpy as np
from scipy.optimize import least_squares
from catalog import OUT


def main():
    old=json.loads((OUT/'log-trap-state-candidate-v14/manifest.json').read_text());rows=np.array(old['surveys']['initial']);observed=rows[:,:4].reshape(7,2,2)+[505,453];radii=rows[:,4];S,C=math.sin(math.radians(35)),math.cos(math.radians(35));bank=43.9491081237793
    prior=json.loads((OUT/'restart2-state/log-triangular-pile-v1/fit.json').read_text());u=np.array(prior['axis']);v=np.array(prior['cross_axis_away_from_camera']);theta=math.atan2(u[1],u[0]);front=[r for r in prior['records']if r['column']==0];cent=np.array([(np.array(r['start'])+r['end'])/2 for r in front]);length=np.array([np.linalg.norm(np.array(r['end'])-r['start'])for r in front])
    def geometry(p):
        theta,s0=p[:2];u=np.array([math.cos(theta),math.sin(theta),0]);v=np.array([-u[1],u[0],0]);levels=[np.c_[s0+np.r_[0,np.cumsum(p[2:8])],np.full(7,bank+radii[0])]];invalid=[]
        for level in range(1,7):
            lower=levels[-1];delta=lower[1:]-lower[:-1];distance=np.linalg.norm(delta,axis=1);mid=(lower[1:]+lower[:-1])/2;radius=radii[level-1]+radii[level];height=np.sqrt(np.maximum(0,radius*radius-distance*distance/4));invalid.extend(np.maximum(0,distance-2*radius));normal=np.c_[-delta[:,1],delta[:,0]]/distance[:,None];levels.append(mid+height[:,None]*normal)
        centers=np.array([level[0,0]*v+p[8+i]*u+np.array([0,0,level[0,1]])for i,level in enumerate(levels)]);ends=np.stack([centers-p[15:,None]*u/2,centers+p[15:,None]*u/2],axis=1);projected=np.stack([ends[:,:,0],-S*ends[:,:,1]-C*ends[:,:,2]],axis=2)
        all_cent=np.concatenate(levels);all_rad=np.concatenate([np.full(len(l),radii[i])for i,l in enumerate(levels)]);dist=np.linalg.norm(all_cent[:,None]-all_cent[None,:],axis=2);ii,jj=np.triu_indices(28,1);overlaps=np.maximum(0,all_rad[ii]+all_rad[jj]-dist[ii,jj]);return levels,ends,projected,u,v,np.r_[invalid,overlaps]
    start=np.r_[theta,cent[0]@v,np.full(6,radii[0]*2+.02),cent@u,length]
    lo=np.r_[-.9,-np.inf,np.full(6,2*radii[0]),np.full(7,-np.inf),np.full(7,60)];hi=np.r_[-.35,np.inf,np.full(6,27),np.full(7,np.inf),np.full(7,170)]
    starts=[start]
    for gap in [20.,23.,26.]:
        other=start.copy();other[2:8]=gap;starts.append(other)
    fits=[least_squares(lambda p:np.r_[(geometry(p)[2]-observed).ravel(),geometry(p)[5]*30],initial,bounds=(lo,hi),max_nfev=700,ftol=1e-10,xtol=1e-10,gtol=1e-10)for initial in starts]
    fit=min(fits,key=lambda f:float(np.sum(f.fun*f.fun)))
    levels,ends,projected,u,v,invalid=geometry(fit.x);records=[]
    for layer,level in enumerate(levels):
        for column,center in enumerate(level):
            c=center[0]*v+fit.x[8+layer]*u+np.array([0,0,center[1]]);records.append(dict(layer=layer,column=column,role='observed near-face log'if column==0 else'inferred supporting course',start=(c-fit.x[15+layer]*u/2).tolist(),end=(c+fit.x[15+layer]*u/2).tolist(),radius=float(radii[layer])))
    dest=OUT/'restart2-state/log-triangular-pile-v3';dest.mkdir(exist_ok=False);report={**prior,'status':'Private CPU irregular supported-course fit; source silhouette and longitudinal support pending','records':records,'axis':u.tolist(),'cross_axis_away_from_camera':v.tolist(),'layer_radii':radii.tolist(),'base_gaps':fit.x[2:8].tolist(),'maximum_circle_overlap':float(invalid.max()),'projected_endpoints':projected.tolist(),'endpoint_error_pixels':np.linalg.norm(projected-observed,axis=2).tolist(),'maximum_endpoint_error_pixels':float(np.linalg.norm(projected-observed,axis=2).max())};(dest/'fit.json').write_text(json.dumps(report,indent=2)+'\n');print(report['maximum_endpoint_error_pixels'],report['maximum_circle_overlap'],report['base_gaps'])
if __name__=='__main__':main()
