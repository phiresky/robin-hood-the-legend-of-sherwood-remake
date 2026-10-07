"""Test a coupled local field with explicit per-cluster non-regression constraints."""
import json,hashlib
from pathlib import Path
import numpy as np
from PIL import Image
from scipy.ndimage import gaussian_filter
from scipy.optimize import minimize
from restart14_tree42_gap_classification import sample,BASE
from restart14_tree42_dense_motion import warp
DEST=BASE/'tree42-anchor-field-v2'
def main():
 DEST.mkdir(exist_ok=False);parent=BASE/'tree42-coherent-correspondence-v2/flows.npz';old=np.load(parent)['flow'].astype(float);plan=json.loads((BASE/'tree42-gap-classification-v2/correction-plan.json').read_text());anchors=[a for a in plan['anchors']if a['eligible_for_bounded_field_trial']];points=np.array([a['source_phase0_center']for a in anchors])-[616.5,688.5];source=json.loads((BASE/'source-reconciliation-v1/report.json').read_text())['groups'][1];features=[]
 for f in source['frames'][:8]:
  x,y,w,h=f['bbox'];a=np.array(Image.open(f['path']).convert('RGBA'),float)/255;c=np.zeros((288,342,4));c[y-688:y-688+h,x-616:x-616+w]=a;c[:,:,:3]*=c[:,:,3:];features.append(gaussian_filter(c,(2,2,0)))
 gy,gx=np.mgrid[-5:6,-5:6];sets=[]
 for q in points:
  xi,yi=np.round(q).astype(int);sets.append((xi+gx,yi+gy))
 yy,xx=np.mgrid[0:288:4,0:342:4];sets.append((xx,yy));radius=6.;data=[]
 for x,y in sets:
  basis=np.exp(-((x[:,:,None]-points[:,0])**2+(y[:,:,None]-points[:,1])**2)/(2*radius**2));data.append((x,y,basis,sample(features[0],x,y),[sample(old[i],x,y)for i in range(8)]))
 def costs(v):
  coeff=v.reshape(7,2);out=[]
  for x,y,basis,target,base in data:
   correction=np.einsum('hwk,kc->hwc',basis,coeff);loss=0.
   for i in range(1,8):
    flow=base[i]+correction*i/7;loss+=np.mean((target-sample(features[i],x+flow[:,:,0],y+flow[:,:,1]))**2)
   out.append(loss)
  return np.array(out)
 zero=np.zeros(14);baseline=costs(zero);ratio=lambda v:costs(v)/baseline;opt=minimize(lambda v:float(ratio(v).sum()),zero,method='SLSQP',bounds=[(-2.5,2.5)]*14,constraints=[{'type':'ineq','fun':lambda v:1-ratio(v)}],options={'maxiter':50,'ftol':1e-9,'disp':True});coeff=opt.x.reshape(7,2);yy,xx=np.mgrid[:288,:342];basis=np.exp(-((xx[:,:,None]-points[:,0])**2+(yy[:,:,None]-points[:,1])**2)/(2*radius**2));correction=np.einsum('hwk,kc->hwc',basis,coeff);weights=np.array(list(np.arange(8)/7)+list(np.arange(6,0,-1)/7));field=old+weights[:,None,None,None]*correction[None];support=features[0][:,:,3]>.08;features+=list(reversed(features[1:7]));rows=[];minjac=1.;folds=0
 for i,a in enumerate(field):
  ddy,ddx=np.gradient(a,axis=(0,1));jac=(1+ddx[:,:,0])*(1+ddy[:,:,1])-ddx[:,:,1]*ddy[:,:,0];minjac=min(minjac,float(jac.min()));folds+=int((jac<=0).sum());before=np.mean((features[0]-warp(features[i],old[i]))**2,axis=2);after=np.mean((features[0]-warp(features[i],a))**2,axis=2);local=[]
  for q in points:
   xi,yi=np.round(q).astype(int);sl=np.s_[yi-5:yi+6,xi-5:xi+6];local.append({'before':float(before[sl].mean()),'after':float(after[sl].mean())})
  rows.append({'phase':i,'global_before':float(before[support].mean()),'global_after':float(after[support].mean()),'local':local})
 local_ratios=[sum(r['local'][j]['after']for r in rows)/sum(r['local'][j]['before']for r in rows)for j in range(7)];global_ratio=sum(r['global_after']for r in rows)/sum(r['global_before']for r in rows);eligible=opt.success and max(local_ratios)<=1+1e-6 and global_ratio<=1 and minjac>.1 and folds==0;assert np.array_equal(field[0],old[0]);assert all(np.array_equal(field[i],field[14-i])for i in range(1,7));out={'status':'CPU_FIELD_CANDIDATE_PENDING_PHYSICAL_VALIDATION'if eligible else 'CPU_CONSTRAINED_TRIAL_HOLD','parent_field_sha256':hashlib.sha256(parent.read_bytes()).hexdigest(),'optimizer_success':bool(opt.success),'optimizer_message':str(opt.message),'radius_pixels':radius,'source_centers_pixel_index':points.tolist(),'correction_coefficients':coeff.tolist(),'phase_weights':weights.tolist(),'local_error_ratios':local_ratios,'global_error_ratio':global_ratio,'minimum_jacobian':minjac,'fold_pixels':folds,'max_motion':float(np.linalg.norm(field,axis=3).max()),'max_step':float(np.linalg.norm(np.roll(field,-1,axis=0)-field,axis=3).max()),'baseline_max_motion':float(np.linalg.norm(old,axis=3).max()),'baseline_max_step':float(np.linalg.norm(np.roll(old,-1,axis=0)-old,axis=3).max()),'rows':rows,'limits':['Coefficients only; no new field archive, Blender, geometry, texture, alpha or neighbor edits.','All14 source phases scored. Per-cluster constraints apply aggregate temporal error, not every phase separately; rows retain any local transient regression.','No physical support improvement or full source parity is claimed without future saved-model validation.']};(DEST/'report.json').write_text(json.dumps(out,indent=2)+'\n');print({k:v for k,v in out.items()if k not in('rows','limits')})
if __name__=='__main__':main()
