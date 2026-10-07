"""Fit a bounded local cyclic field using source-consistent cluster anchors."""
from pathlib import Path
import json,hashlib
import numpy as np
from PIL import Image
from scipy.ndimage import gaussian_filter
from scipy.optimize import least_squares
from restart14_tree42_gap_classification import sample,BASE
from restart14_tree42_dense_motion import warp
DEST=BASE/'tree42-anchor-field-v1';sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 DEST.mkdir(exist_ok=False);plan_path=BASE/'tree42-gap-classification-v2/correction-plan.json';plan=json.loads(plan_path.read_text());anchors=[a for a in plan['anchors']if a['eligible_for_bounded_field_trial']];source=json.loads((BASE/'source-reconciliation-v1/report.json').read_text())['groups'][1];packet=np.load(BASE/'tree42-coherent-correspondence-v2/flows.npz');old=packet['flow'].astype(float);features=[]
 for f in source['frames']:
  x,y,w,h=f['bbox'];a=np.array(Image.open(f['path']).convert('RGBA'),dtype=float)/255;c=np.zeros((288,342,4));c[y-688:y-688+h,x-616:x-616+w]=a;c[:,:,:3]*=c[:,:,3:];features.append(gaussian_filter(c,(2,2,0)))
 gy,gx=np.mgrid[-5:6,-5:6];weight=np.exp(-(gx*gx+gy*gy)/18);weight=np.sqrt(weight/weight.sum());fits=[];points=[];target_vectors=[]
 for a in anchors:
  q=np.array(a['source_phase0_center'])-[616.5,688.5];initial=np.array(a['forward_displacement']);target=sample(features[0],q[0]+gx,q[1]+gy)
  def residual(v):
   errors=[((sample(features[i],q[0]+gx+v[0]*i/7,q[1]+gy+v[1]*i/7)-target)*weight[:,:,None]).ravel()for i in range(1,8)]
   errors.append((v-initial)*.012);return np.concatenate(errors)
  fit=least_squares(residual,initial,bounds=(initial-.75,initial+.75),max_nfev=60,ftol=1e-9,xtol=1e-9,gtol=1e-9);points.append(q);target_vectors.append(fit.x);fits.append({'native_target_pixel':a['native_target_pixel'],'source_center':a['source_phase0_center'],'phase7_bidirectional_vector':initial.tolist(),'all_phase_fitted_vector':fit.x.tolist(),'endpoint_adjustment':float(np.linalg.norm(fit.x-initial)),'least_squares_cost':float(fit.cost),'converged':bool(fit.success)})
 points=np.array(points);vectors=np.array(target_vectors);base_vectors=np.array([sample(old[7],q[0],q[1])for q in points]);yy,xx=np.mgrid[:288,:342];support=features[0][:,:,3]>.08;trials=[]
 for radius in(6.,9.,12.):
  kernel=np.exp(-np.sum((points[:,None,:]-points[None,:,:])**2,axis=2)/(2*radius**2));coeff=np.linalg.solve(kernel+np.eye(len(points))*.02,vectors-base_vectors);basis=np.exp(-((xx[:,:,None]-points[:,0])**2+(yy[:,:,None]-points[:,1])**2)/(2*radius**2));correction=np.einsum('hwk,kc->hwc',basis,coeff)
  for strength in(1.,.75,.5,.25):
   weights=np.array(list(np.arange(8)/7)+list(np.arange(6,0,-1)/7));field=old+weights[:,None,None,None]*strength*correction[None];rows=[];minimum=1.;folds=0
   for i,a in enumerate(field):
    ddy,ddx=np.gradient(a,axis=(0,1));jac=(1+ddx[:,:,0])*(1+ddy[:,:,1])-ddx[:,:,1]*ddy[:,:,0];minimum=min(minimum,float(jac.min()));folds+=int((jac<=0).sum());before=np.mean((features[0]-warp(features[i],old[i]))**2,axis=2);after=np.mean((features[0]-warp(features[i],a))**2,axis=2);local=[]
    for q in points:
     xi=int(round(q[0]));yi=int(round(q[1]));crop=np.s_[yi-5:yi+6,xi-5:xi+6];local.append({'before':float(before[crop].mean()),'after':float(after[crop].mean())})
    rows.append({'phase':i,'global_before':float(before[support].mean()),'global_after':float(after[support].mean()),'local':local})
   local_before=sum(v['before']for r in rows for v in r['local']);local_after=sum(v['after']for r in rows for v in r['local']);global_before=sum(r['global_before']for r in rows);global_after=sum(r['global_after']for r in rows);eligible=minimum>.1 and folds==0 and global_after<=global_before and local_after<local_before;trials.append({'radius':radius,'strength':strength,'minimum_jacobian':minimum,'folds':folds,'global_error_change_fraction':global_after/global_before-1,'local_error_change_fraction':local_after/local_before-1,'eligible':eligible,'max_motion':float(np.linalg.norm(field,axis=3).max()),'max_step':float(np.linalg.norm(np.roll(field,-1,axis=0)-field,axis=3).max()),'rows':rows});print({k:v for k,v in trials[-1].items()if k!='rows'},flush=True)
 eligible=[t for t in trials if t['eligible']];chosen=min(eligible,key=lambda t:t['local_error_change_fraction'])if eligible else None
 if chosen:
  radius=chosen['radius'];kernel=np.exp(-np.sum((points[:,None,:]-points[None,:,:])**2,axis=2)/(2*radius**2));coeff=np.linalg.solve(kernel+np.eye(len(points))*.02,vectors-base_vectors);basis=np.exp(-((xx[:,:,None]-points[:,0])**2+(yy[:,:,None]-points[:,1])**2)/(2*radius**2));field=old+weights[:,None,None,None]*chosen['strength']*np.einsum('hwk,kc->hwc',basis,coeff)[None];assert np.array_equal(field[0],old[0]);assert all(np.array_equal(field[i],field[14-i])for i in range(1,7));np.savez_compressed(DEST/'flows.npz',flow=field.astype('float32'),bbox=packet['bbox'])
 out={'status':'CPU_FIELD_CANDIDATE_PENDING_GEOMETRY_REVIEW'if chosen else 'NO_ELIGIBLE_FIELD_TRIAL','prototype_unchanged':True,'parent_field_sha256':sha(BASE/'tree42-coherent-correspondence-v2/flows.npz'),'constraints_sha256':sha(plan_path),'anchor_fits':fits,'chosen':chosen,'trials':trials,'limitations':['No geometry rerender or alpha coverage improvement is claimed.','Only seven previously bidirectionally checked anchors constrain this experiment; excluded three remain excluded.','All14 own-source frames enter error comparison; linear palindrome phase weights preserve native phase0/loop and4tick holds.','Hidden/rear motion remains inferred; no texture, alpha, wood or neighbor edits.']};(DEST/'report.json').write_text(json.dumps(out,indent=2)+'\n')
if __name__=='__main__':main()
