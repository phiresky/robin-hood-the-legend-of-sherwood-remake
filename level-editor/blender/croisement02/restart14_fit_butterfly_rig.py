"""Fit three observed poses using one conserved body and hinged wing anatomy."""
from pathlib import Path
import json,math,hashlib
import numpy as np
from PIL import Image
from scipy.optimize import differential_evolution
from scipy.spatial import ConvexHull
from scipy.spatial.transform import Rotation
from scipy.ndimage import distance_transform_edt
from matplotlib.path import Path as PolygonPath
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/croisement02-refinement/restart14-butterflies';OUT=BASE/'rig-v1';OUT.mkdir(exist_ok=True)
source=json.loads((BASE/'source-audit-v1/report.json').read_text())['sequences'][0]
# One fixed outline per side: forewing and hindwing lobes are intentionally modest.
outline=np.array([[0,-2.2],[1.0,-3.5],[4.7,-5.6],[6.1,-5.8],[6.5,-3.2],[5.6,-.2],[3.7,1.2],[4.2,3.6],[2.0,4.8],[.3,2.4],[0,1.3]],float)
# Round the outline without changing it between phases.
for _ in range(2):outline=np.concatenate([np.stack((.75*outline+.25*np.roll(outline,-1,axis=0),.25*outline+.75*np.roll(outline,-1,axis=0)),axis=1).reshape(-1,2)])
wing=np.column_stack((outline,.16*np.sin(np.pi*outline[:,0]/6.5)))
body=np.array([[.36*math.sin(a)*math.cos(t),4.5*math.cos(a),.55*math.sin(a)*math.sin(t)] for a in np.linspace(0,math.pi,13) for t in np.linspace(0,math.tau,25)[:-1]])
def pose(params):
 rx,ry,rz,left,right,tx,ty=params;global_rot=Rotation.from_euler('xyz',[rx,ry,rz],degrees=True).as_matrix()
 wings=[]
 for sign,angle in [(-1,left),(1,right)]:
  pts=wing.copy();pts[:,0]*=sign;rot=Rotation.from_euler('y',-sign*angle,degrees=True).as_matrix();pts=pts@rot.T;pts[:,0]+=sign*.30;wings.append(pts@global_rot.T)
 return body@global_rot.T,wings,np.array([tx,ty])
fits=[]
for phase in [0,2,18]:
 f=source['frames'][phase];a=np.array(Image.open(f['source']).convert('RGBA'));mask=a[:,:,3]>0;h,w=mask.shape;ys,xs=np.nonzero(mask);center=np.array([xs.mean()+.5,ys.mean()+.5]);gy,gx=np.mgrid[-3:h+3,-3:w+3];queries=np.column_stack((gx.ravel()+.5,gy.ravel()+.5));target=np.zeros(gx.shape,bool);target[3:3+h,3:3+w]=mask;target=target.ravel();outside_distance=distance_transform_edt(~target.reshape(gx.shape)).ravel();weights=np.ones(gx.shape);brightness=np.max(a[:,:,:3],axis=2)/255;weights[3:3+h,3:3+w]=.6+brightness;weights=weights.ravel()
 def raster(p):
  b,ws,shift=pose(p);bxy=b[:,:2]+center+shift;result=PolygonPath(bxy[ConvexHull(bxy).vertices]).contains_points(queries)
  for q in ws:result|=PolygonPath(q[:,:2]+center+shift).contains_points(queries)
  return result
 def objective(p):
  pred=raster(p);miss=np.sum(weights[target&~pred]);extra=np.sum(1+.25*outside_distance[pred&~target]);regular=.012*((p[3]-p[4])**2)+.003*(p[0]**2+p[1]**2)
  return (miss+extra+regular)/target.sum()
 if phase==0:bounds=[(-15,15),(-15,15),(-12,12),(35,65),(35,65),(-1.5,1.5),(-1.5,1.5)]
 elif phase==2:bounds=[(-15,15),(-15,15),(-12,12),(0,28),(0,28),(-1.5,1.5),(-1.5,1.5)]
 else:bounds=[(-20,20),(-15,15),(-70,-20),(70,88),(70,88),(-1.5,1.5),(-1.5,1.5)]
 result=differential_evolution(objective,bounds,seed=phase+410,maxiter=80,popsize=10,polish=False,tol=.005);pred=raster(result.x)
 # Texture inference is bounded to the same phase's own observed RGB.
 _,near=distance_transform_edt(~mask,return_indices=True);filled=a.copy();filled[~mask,:3]=a[near[0][~mask],near[1][~mask],:3];filled[:,:,3]=255;texture=OUT/f'phase-{phase:02d}-own-source-fill.png';Image.fromarray(filled).save(texture)
 b,ws,shift=pose(result.x);row={'phase':phase,'source':f,'source_center':center.tolist(),'parameters':result.x.tolist(),'parameter_order':['body_rx','body_ry','body_rz','left_hinge_deg','right_hinge_deg','source_dx','source_dy'],'source_centers':int(target.sum()),'covered_source_centers':int((target&pred).sum()),'missing_source_centers':int((target&~pred).sum()),'extra_centers':int((~target&pred).sum()),'iou':float((target&pred).sum()/(target|pred).sum()),'texture':str(texture),'texture_sha256':hashlib.sha256(texture.read_bytes()).hexdigest(),'source_positive_rgba_retained':bool(np.array_equal(filled[mask],a[mask])),'inferred_altitude':60}
 fits.append(row);print(phase,row['parameters'],row['covered_source_centers'],row['missing_source_centers'],row['extra_centers'],flush=True)
packet={'status':'PRIVATE_THREE_PHASE_CONSERVED_RIG_PROTOTYPE','source_sequence':8,'source_cycle_ticks':198,'wing_outline':wing.tolist(),'body_radii':[.36,4.5,.55],'wing_thickness':.085,'poses':fits,'constraints':['Only rigid body orientation and left/right hinge rotation change pose; no per-phase vertex morphing or silhouette cutout meshes.','Phase0 partially folded, phase2 open, phase18 strongly closed bounds are source-informed hypotheses. Hidden depth/altitude are inferred.','Source RGB preserved in projected image where covered. Fixed anatomy cannot promise exact source pixel silhouette; missing/extra centers explicitly measured.','Own source fills unknown wing RGB; no API or external reference.','Only three representative poses;99 source frames/timing retained as authority, full motion remains unfitted.']}
(OUT/'fit.json').write_text(json.dumps(packet,indent=2)+'\n')
