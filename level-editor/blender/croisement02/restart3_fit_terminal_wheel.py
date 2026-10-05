import json,math
from pathlib import Path
import numpy as np
from PIL import Image
from scipy.optimize import differential_evolution
r=Path(__file__).resolve().parents[2]/'work/croisement02-refinement';a=np.array(Image.open(r/'state-target-evidence/profiles/chariot03-10/action-160-direction-0-frame-099.png'));yy,xx=np.mgrid[100:135,99:135];x=xx+.5;y=yy+.5;src=a[100:135,99:135];truth=src[:,:,3]>0;bright=src[:,:,:3].max(axis=2)>65
# Strong evidence: transparent spoke holes, opaque warm timber. Very dark outer
# pixels may be cast shadow or timber and contribute only weakly.
weight=np.where(truth,np.where(bright,1.0,.15),1.5);weight[(yy<107)&(xx<121)]=0;weight[(xx<104)&(yy>116)]=.05
S=np.sin(np.deg2rad(35));C=np.cos(np.deg2rad(35));rows=[]
for n in [6,8,10,12]:
 def loss(q):
  cx,cy,rad,theta,phase,width,rim=q;aa=(x-cx)/np.cos(theta);bb=-(y-cy+aa*np.sin(theta)*S)/C;dist=np.hypot(aa,bb);angle=np.arctan2(bb,aa)
  nearest=(angle-phase+np.pi/n)%(2*np.pi/n)-np.pi/n
  spokes=(np.abs(np.sin(nearest)*dist)<width/2)&(dist<rad-rim+.6)
  pred=((dist<=rad)&(dist>=rad-rim))|spokes|(dist<3.3)
  return float((weight*(pred!=truth)).sum())
 fit=differential_evolution(loss,[(114,122),(112,120),(17,24),(.4,1.1),(0,2*np.pi/n),(1.2,3.2),(2,5)],popsize=10,maxiter=90,seed=731+n,polish=False)
 row={'spokes':n,'weighted_mismatches':float(fit.fun),'parameters':fit.x.tolist()};rows.append(row);print(row,flush=True)
p=r/'restart3-north-cart/terminal-physical-v6/source-front-v1/wheel-fit-proposals.json';p.write_text(json.dumps({'scope':'CPU analytic silhouette proposals only; no geometry mutation or source ownership reclassification','parameters':['cx','cy','radius','vertical-plane-axis-angle','spoke-phase','spoke-width','rim-width'],'rows':rows,'caveat':'Dark silhouette pixels and top-left body overlap are downweighted; finite rim width, hub projection and physical contact need saved-model review.'},indent=2)+'\n')
