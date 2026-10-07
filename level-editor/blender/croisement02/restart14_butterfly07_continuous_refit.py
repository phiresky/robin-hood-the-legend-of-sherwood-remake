"""Bound phase20 body direction by adjacent own-source poses, then test contacts."""
import copy,json
import numpy as np
from PIL import Image
from scipy.ndimage import distance_transform_edt
from scipy.optimize import differential_evolution
from scipy.spatial import ConvexHull
from scipy.spatial.transform import Rotation,Slerp
from matplotlib.path import Path as Polygon
from restart14_butterfly07_pose_fit import geometry
import restart14_butterfly07_fit_alternatives as audit
B=audit.B;OUT=B/'butterfly07-continuous-refit-v1'

def main():
    assert not OUT.exists();OUT.mkdir()
    source=B/'butterfly07-contact-free-selection-v1/fit.json';fit=json.loads(source.read_text());rows={r['phase']:r for r in fit['rows']};old=rows[20]
    reference=Slerp([0,1],Rotation.from_euler('xyz',[rows[19]['parameters'][:3],rows[21]['parameters'][:3]],degrees=True))([.5])[0]
    rgba=np.asarray(Image.open(old['source']['source']).convert('RGBA'));mask=rgba[:,:,3]>0;h,w=mask.shape;y,x=np.nonzero(mask);center=np.array([x.mean()+.5,y.mean()+.5]);gy,gx=np.mgrid[-3:h+3,-3:w+3];points=np.c_[gx.ravel()+.5,gy.ravel()+.5];target=np.zeros(gx.shape,bool);target[3:h+3,3:w+3]=mask;distance=distance_transform_edt(~target).ravel();target=target.ravel()
    def evaluate(q):
        delta=Rotation.from_rotvec(np.deg2rad(q[:3]));rotation=reference*delta;p=np.r_[rotation.as_euler('xyz',degrees=True),q[3:]];body,wings=geometry(p);shift=center+p[5:7];b=body[:,:2]+shift;pred=Polygon(b[ConvexHull(b).vertices]).contains_points(points)
        for wing in wings:pred|=Polygon(wing[:,:2]+shift).contains_points(points)
        missing=int((target&~pred).sum());extra=int((pred&~target).sum());angle=float(delta.magnitude()*180/np.pi)
        loss=(missing+(1+.2*distance[pred&~target]).sum())/target.sum()+.012*float(np.sum(p[5:7]**2))+.05*(angle/25)**2+max(0,angle-25)*10
        return {'parameters':p.tolist(),'covered':int((target&pred).sum()),'missing':missing,'source_pixels':int(target.sum()),'extra':extra,'loss':float(loss),'body_reference_delta_degrees':angle,'depth_mirrored':False,'quaternion':rotation.as_quat().tolist()}
    candidates=[]
    for seed in range(8):
        result=differential_evolution(lambda q:evaluate(q)['loss'],[(-15,15)]*3+[(-88,88),(-88,88),(-2,2),(-2,2)],seed=7300+seed,popsize=7,maxiter=100,polish=False,tol=.003)
        ranked=sorted((evaluate(q)for q in result.population),key=lambda c:c['loss'])
        for c in ranked[:3]:
            if not any(np.linalg.norm(np.array(c['parameters'])-r['parameters'])<.05 for r in candidates):c['seed']=7300+seed;candidates.append(c)
        print('seed',seed,'best',ranked[0]['covered'],ranked[0]['extra'],ranked[0]['body_reference_delta_degrees'],flush=True)
    proposal=copy.deepcopy(fit);proposal.update(status='BOUNDED_PHASE20_BODY_DIRECTION_REFIT',rows=[old],candidate_banks={'20':candidates},parent_fit_sha256=audit.reader.sha(source),refit_recipe_sha256=audit.reader.sha(__import__('pathlib').Path(__file__)),body_reference_quaternion=reference.as_quat().tolist(),body_direction_bound_degrees=25,observed_vs_inferred='Phase20 source pixels observed. Body-direction continuity and hidden wing depth inferred; no new observed interpolation frames.')
    (OUT/'fit.json').write_text(json.dumps(proposal,indent=2)+'\n');audit.main(OUT/'fit.json',B/'butterfly07-continuous-refit-contacts-v1')

if __name__=='__main__':main()
