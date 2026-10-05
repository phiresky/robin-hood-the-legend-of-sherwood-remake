"""Fit a supported triangular log pile to the observed near-face cap centers."""
import json,math,hashlib
from pathlib import Path
import numpy as np
from scipy.optimize import least_squares
from catalog import OUT


def main():
    base=OUT/'log-trap-state-candidate-v14';old=json.loads((base/'manifest.json').read_text());rows=np.array(old['surveys']['initial']);observed=rows[:,:4].reshape(7,2,2)+np.array([505,453]);S=math.sin(math.radians(35));C=math.cos(math.radians(35));bank=43.9491081237793
    theta=math.atan2(-.555803,.831314);u=np.array([math.cos(theta),math.sin(theta),0]);v=np.array([-u[1],u[0],0]);z=bank+7+np.arange(7)*math.sqrt(3)*7
    xy=np.c_[observed[:,:,0].mean(axis=1),-(observed[:,:,1].mean(axis=1)+z*C)/S,z];ts=xy@u;s0=float((xy@v-np.arange(7)*7).mean());lengths=(observed[:,1,0]-observed[:,0,0])/u[0]
    def geometry(params):
        theta,r,s0=params[:3];u=np.array([math.cos(theta),math.sin(theta),0]);v=np.array([-u[1],u[0],0]);centers=params[3:10,None]*u+(s0+np.arange(7)*r)[:,None]*v;centers[:,2]=bank+r+np.arange(7)*math.sqrt(3)*r
        ends=np.stack([centers-params[10:,None]*u/2,centers+params[10:,None]*u/2],axis=1)
        projected=np.stack([ends[:,:,0],-S*ends[:,:,1]-C*ends[:,:,2]],axis=2)
        return centers,ends,projected,u,v
    start=np.r_[theta,7,s0,ts,lengths]
    def residual(params):return np.r_[(geometry(params)[2]-observed).ravel(),(params[1]-rows[:,4].mean())*2]
    fit=least_squares(residual,start,bounds=(np.r_[-.9,5,-np.inf,np.full(7,-np.inf),np.full(7,70)],np.r_[-.35,9,np.inf,np.full(7,np.inf),np.full(7,160)]),xtol=1e-12,ftol=1e-12,gtol=1e-12,max_nfev=1000)
    centers,ends,projected,u,v=geometry(fit.x);records=[]
    for layer in range(7):
        for column in range(7-layer):
            shift=v*(2*fit.x[1]*column);records.append(dict(layer=layer,column=column,role='observed near-face log'if column==0 else'inferred hidden support course',start=(ends[layer,0]+shift).tolist(),end=(ends[layer,1]+shift).tolist(),radius=float(fit.x[1])))
    dest=OUT/'restart2-state/log-triangular-pile-v1';dest.mkdir(exist_ok=False)
    report=dict(status='Private supported-pile fit; source alignment, bank contact and full solid review pending',source_model_sha256=hashlib.sha256((base/'worker.blend').read_bytes()).hexdigest(),bank_model=str(OUT/'restart2-bank321/packaged-v1/assets/croisement02-north-woodland-bank/model.blend'),bank_model_sha256='69ecb7b704e30d6d64565a44aa810a21b924195609dbe7ac35818a0209137641',plateau_height=bank,source_endpoints=observed.tolist(),projected_endpoints=projected.tolist(),endpoint_error_pixels=np.linalg.norm(projected-observed,axis=2).tolist(),maximum_endpoint_error_pixels=float(np.linalg.norm(projected-observed,axis=2).max()),radius=float(fit.x[1]),axis=u.tolist(),cross_axis_away_from_camera=v.tolist(),records=records,layer_counts=[7,6,5,4,3,2,1],support='Circular hex packing: every upper log rests between two lower courses. Axis extents vary to fit native observed cap positions; longitudinal overlap and exact solids require audit.',limitations=['Seven visible near-face logs are source evidence;21 hidden supporting logs are inferred.','All fallen endpoint geometry must remain exact.','Equal-radius initial prototype; native cap-fit error is measured, not claimed exact.'])
    (dest/'fit.json').write_text(json.dumps(report,indent=2)+'\n');print(report['maximum_endpoint_error_pixels'],report['radius'],report['layer_counts'])
if __name__=='__main__':main()
