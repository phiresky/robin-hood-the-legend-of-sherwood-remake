"""Diagnose butterfly pose ambiguity with source-space anatomical landmarks.

CPU-only private hypotheses: no scene, source image, or model is changed.
"""
from pathlib import Path
import hashlib,json,math
import numpy as np
from PIL import Image,ImageDraw
from scipy.ndimage import distance_transform_edt
from scipy.optimize import differential_evolution
from scipy.spatial import ConvexHull
from scipy.spatial.transform import Rotation
from matplotlib.path import Path as PolygonPath

ROOT=Path(__file__).resolve().parents[3]
BASE=ROOT/'level-editor/work/croisement02-refinement/restart14-butterflies'
OUT=BASE/'pose-landmarks-v1'
# Manually reviewed original pixel coordinates; these have roughly one-pixel
# uncertainty, and are correspondence hypotheses rather than new source art.
LANDMARKS={
18:{'tip_mean':[1.5,.75],'hind_mean':[8.,6.],'axis':[.78,.63]},
27:{'tips':[[2.,1.5],[11.,1.5]],'center':[7.,5.],'axis':[0.,1.]},
28:{'tips':[[1.,1.],[7.5,1.5]],'center':[4.5,6.5],'axis':[0.,1.]},
29:{'tips':[[1.5,2.5],[9.5,1.5]],'center':[6.,5.],'axis':[0.,1.]},
30:{'tips':[[2.5,3.5],[10.,1.5]],'center':[6.,4.],'axis':[.3,1.]},
}

def main():
 OUT.mkdir(exist_ok=True)
 packet=json.loads((BASE/'rig-full-v2/fit.json').read_text())
 wing=np.array(packet['wing_outline']);tip_index=int(np.argmin(wing[:,1]));hind_index=int(np.argmax(wing[:,1]))
 body=np.array([[.36*math.sin(a)*math.cos(t),4.5*math.cos(a),.55*math.sin(a)*math.sin(t)] for a in np.linspace(0,math.pi,13) for t in np.linspace(0,math.tau,25)[:-1]])
 def geometry(p):
  g=Rotation.from_euler('xyz',p[:3],degrees=True).as_matrix();w=[]
  for sign,angle in [(-1,p[3]),(1,p[4])]:
   v=wing.copy();v[:,0]*=sign;v=Rotation.from_euler('y',-sign*angle,degrees=True).apply(v);v[:,0]+=sign*.30;w.append(v@g.T)
  return body@g.T,w,g[:,1]
 reports=[];sheet=Image.new('RGB',(960,260*len(LANDMARKS)),'#252525');draw=ImageDraw.Draw(sheet)
 for ri,(phase,lm) in enumerate(LANDMARKS.items()):
  row=packet['poses'][phase];im=Image.open(row['source']['source']).convert('RGBA');rgba=np.array(im);mask=rgba[:,:,3]>0;h,w=mask.shape;ys,xs=np.nonzero(mask);center=np.array([xs.mean()+.5,ys.mean()+.5]);gy,gx=np.mgrid[-4:h+4,-4:w+4];q=np.column_stack((gx.ravel()+.5,gy.ravel()+.5));target=np.zeros(gx.shape,bool);target[4:4+h,4:4+w]=mask;target=target.ravel();dist=distance_transform_edt(~target.reshape(gx.shape)).ravel();weights=np.ones(gx.shape);weights[4:4+h,4:4+w]=.6+np.max(rgba[:,:,:3],axis=2)/255;weights=weights.ravel()
  def evaluate(p):
   b,ws,axis=geometry(p);shift=center+np.array(p[5:]);b=b[:,:2]+shift;ws=[v[:,:2]+shift for v in ws];pred=PolygonPath(b[ConvexHull(b).vertices]).contains_points(q)
   for v in ws:pred|=PolygonPath(v).contains_points(q)
   tips=np.array([v[tip_index] for v in ws]);hind=np.array([v[hind_index] for v in ws]);axis=axis[:2]/max(np.linalg.norm(axis[:2]),1e-8);landmark_loss=0.
   if 'tips' in lm:landmark_loss+=np.sum((tips-np.array(lm['tips']))**2)
   if 'center' in lm:landmark_loss+=.7*np.sum((shift-np.array(lm['center']))**2)
   if 'tip_mean' in lm:landmark_loss+=2*np.sum((tips.mean(axis=0)-lm['tip_mean'])**2)
   if 'hind_mean' in lm:landmark_loss+=2*np.sum((hind.mean(axis=0)-lm['hind_mean'])**2)
   intended=np.array(lm['axis']);intended/=np.linalg.norm(intended);landmark_loss+=8*np.sum((axis-intended)**2)
   silhouette=float(weights[target&~pred].sum()+(1+.25*dist[pred&~target]).sum())
   return silhouette,landmark_loss,pred,ws,b,{'tips':tips.tolist(),'hind_mean':hind.mean(axis=0).tolist(),'root_center':shift.tolist(),'body_axis':axis.tolist()}
  def loss(p):
   silhouette,landmark,*_=evaluate(p)
   return silhouette+2.5*landmark+.01*(p[3]-p[4])**2
  fits=[]
  for seed in [1103,2103]:
   fit=differential_evolution(loss,[(-80,80),(-80,80),(-90,90),(0,88),(0,88),(-4,4),(-4,4)],seed=seed+phase,popsize=12,maxiter=100,polish=False,tol=.004);fits.append(fit)
  best=min(fits,key=lambda f:f.fun)
  versions={}
  for col,(name,p) in enumerate([('Source',None),('Previous',row['parameters']),('Landmark hypothesis',best.x)]):
   ox=col*320+45;oy=ri*260+50;scale=12;enlarged=im.resize((w*scale,h*scale),Image.Resampling.NEAREST);sheet.paste(enlarged,(ox,oy),enlarged);draw.text((col*320+8,ri*260+8),f'Phase{phase}: {name}',fill='white')
   point=lambda v:(ox+float(v[0])*scale,oy+float(v[1])*scale)
   if p is not None:
    sil,land,pred,ws,b,marks=evaluate(p)
    for poly,color in zip(ws,['#00e5ff','#ff63ff']):draw.line([point(v) for v in np.vstack((poly,poly[0]))],fill=color,width=2)
    hull=b[ConvexHull(b).vertices];draw.line([point(v) for v in np.vstack((hull,hull[0]))],fill='#ffa800',width=2)
    for tip in marks['tips']:
     x,y=point(tip);draw.ellipse((x-3,y-3,x+3,y+3),fill='#ffffff')
    versions[name]={'parameters':np.array(p).tolist(),'covered':int((pred&target).sum()),'missed':int((~pred&target).sum()),'extra':int((pred&~target).sum()),'silhouette_loss':sil,'anatomical_landmark_loss':float(land),'projected_landmarks':marks}
    draw.text((col*320+8,ri*260+220),f"covered{versions[name]['covered']}/{int(target.sum())} extra{versions[name]['extra']}",fill='white')
   else:
    points=lm.get('tips',[])+[lm[k] for k in ['center','tip_mean','hind_mean'] if k in lm]
    for v in points:
     x,y=point(v);draw.ellipse((x-5,y-5,x+5,y+5),outline='#ffb000',width=2)
  reports.append({'phase':phase,'source_sha256':row['source']['sha256'],'source_positive_pixels':int(target.sum()),'manual_source_landmarks':lm,'landmark_uncertainty_pixels':1.,'versions':versions,'seeds':[{'seed':seed+phase,'objective':float(f.fun)} for seed,f in zip([1103,2103],fits)]});print('LANDMARK_FIT',phase,versions['Landmark hypothesis']['covered'],versions['Landmark hypothesis']['missed'],flush=True)
 sheet.save(OUT/'source-previous-landmark-proposal.png')
 report={'status':'CPU_ONLY_POSE_PROPOSAL_NOT_SAVED_OR_RENDERED','parent_model_sha256':hashlib.sha256((BASE/'rig-full-v2/model.blend').read_bytes()).hexdigest(),'diagnosis':['The previous silhouette-only objective admits rotated stacked wings with similar alpha coverage but wrong anatomical correspondence.','Sequential35degree body search bounds carry a sideways local minimum into source-open phases27-30.','Phase18 inherited an overconstrained early anchor; subsequent smoothing changes its body rotation without re-solving hinge/registration.','This proposal removes local attitude bounds and scores manually inspected forewing tips, root centers and projected body orientation in addition to silhouette.'],'unchanged':['rest anatomy','fixed own-phase2 image and UV','99 phases and198tick timing','source PNGs','all existing models'], 'rows':reports,'limitations':['Manual landmarks carry one-pixel uncertainty and are hypotheses at this tiny source resolution.','No full99 re-fit, saved-model review, temporal continuity or scene occlusion acceptance is claimed.','Both global seed results retained to identify residual local minima.']}
 (OUT/'landmark-fit-proposal.json').write_text(json.dumps(report,indent=2)+'\n')
if __name__=='__main__':main()
