"""Refine full rigid butterfly attitude without changing anatomical geometry."""
from pathlib import Path
import json,math,hashlib
import numpy as np
from PIL import Image
from scipy.optimize import differential_evolution
from scipy.spatial import ConvexHull
from scipy.spatial.transform import Rotation
from scipy.ndimage import distance_transform_edt,gaussian_filter1d
from matplotlib.path import Path as PolygonPath
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/croisement02-refinement/restart14-butterflies';OUT=BASE/'rig-full-v2';OUT.mkdir(exist_ok=True)
source=json.loads((BASE/'source-audit-v1/report.json').read_text())['sequences'][0];ref=json.loads((BASE/'rig-v1/fit.json').read_text());wing=np.array(ref['wing_outline']);body=np.array([[.36*math.sin(a)*math.cos(t),4.5*math.cos(a),.55*math.sin(a)*math.sin(t)] for a in np.linspace(0,math.pi,13) for t in np.linspace(0,math.tau,25)[:-1]])
anchors={r['phase']:r for r in ref['poses']}
rollkeys=np.array([[0,-10.85],[2,-8.31],[18,-29.13],[29,0],[37,30],[53,0],[65,35],[75,-5],[84,-35],[98,-10.85]])
def posed(params):
 rx,ry,rz,left,right,tx,ty=params;g=Rotation.from_euler('xyz',[rx,ry,rz],degrees=True).as_matrix();ws=[]
 for sign,angle in [(-1,left),(1,right)]:
  v=wing.copy();v[:,0]*=sign;v=v@Rotation.from_euler('y',-sign*angle,degrees=True).as_matrix().T;v[:,0]+=sign*.30;ws.append(v@g.T)
 return body@g.T,ws,np.array([tx,ty])
def context(f):
 a=np.array(Image.open(f['source']));mask=a[:,:,3]>0;h,w=mask.shape;ys,xs=np.nonzero(mask);center=np.array([xs.mean()+.5,ys.mean()+.5]);gy,gx=np.mgrid[-3:h+3,-3:w+3];q=np.column_stack((gx.ravel()+.5,gy.ravel()+.5));target=np.zeros(gx.shape,bool);target[3:3+h,3:3+w]=mask;target=target.ravel();dist=distance_transform_edt(~target.reshape(gx.shape)).ravel();weights=np.ones(gx.shape);weights[3:3+h,3:3+w]=.6+np.max(a[:,:,:3],axis=2)/255;weights=weights.ravel()
 def raster(params):
  b,ws,shift=posed(params);xy=b[:,:2]+center+shift;result=PolygonPath(xy[ConvexHull(xy).vertices]).contains_points(q)
  for v in ws:result|=PolygonPath(v[:,:2]+center+shift).contains_points(q)
  return result
 return center,target,dist,weights,raster
rows=[]
for f in source['frames']:
 phase=f['index'];center,target,dist,weights,raster=context(f);roll=float(np.interp(phase,rollkeys[:,0],rollkeys[:,1]))
 if phase in anchors:params=np.array(anchors[phase]['parameters'])
 elif phase==98:params=np.array(anchors[0]['parameters'])
 else:
  def loss(p):
   pred=raster(p);previous=np.array(rows[-1]['parameters'][:3]) if rows else np.zeros(3);reg=.008*(p[3]-p[4])**2+.0006*np.sum((p[:3]-previous)**2)+.0002*(p[2]-roll)**2
   return (weights[target&~pred].sum()+(1+.25*dist[pred&~target]).sum()+reg)/target.sum()
  previous=np.array(rows[-1]['parameters'][:3]) if rows else np.zeros(3); bounds=[(max(-70,previous[0]-35),min(70,previous[0]+35)),(max(-70,previous[1]-35),min(70,previous[1]+35)),(max(-90,previous[2]-35),min(90,previous[2]+35)),(0,88),(0,88),(-2.5,2.5),(-2.5,2.5)]
  result=differential_evolution(loss,bounds,seed=phase+803,popsize=9,maxiter=60,polish=False,tol=.01);params=result.x
 rows.append({'phase':phase,'source':f,'source_center':center.tolist(),'parameters':params.tolist(),'inferred_altitude':60.0})
 if phase%20==0:print('FIT',phase,flush=True)
# Smooth hidden body orientation only; wing opening follows every observed source phase.
array=np.array([r['parameters'] for r in rows]);array[:,:3]=gaussian_filter1d(array[:,:3],sigma=.55,axis=0,mode='wrap');array[98]=array[0]
for row,params in zip(rows,array):
 center,target,dist,weights,raster=context(row['source']);
 if row['phase'] not in (0,2,18,98):
  fixed=params[:3].copy()
  def hinge_loss(x):
   candidate=np.r_[fixed,x];pred=raster(candidate);return (weights[target&~pred].sum()+(1+.25*dist[pred&~target]).sum()+.008*(x[0]-x[1])**2)/target.sum()
  result=differential_evolution(hinge_loss,[(0,88),(0,88),(-2.5,2.5),(-2.5,2.5)],seed=row['phase']+911,popsize=7,maxiter=35,polish=False,tol=.01);params[3:]=result.x
 row['parameters']=params.tolist();pred=raster(params);row.update(source_centers=int(target.sum()),covered_source_centers=int((target&pred).sum()),missing_source_centers=int((target&~pred).sum()),extra_centers=int((~target&pred).sum()),iou=float((target&pred).sum()/(target|pred).sum()))
packet={**ref,'status':'PRIVATE_FULL99_CONSERVED_RIG','poses':rows,'source_clock':[{'phase':f['index'],'first_tick':f['first_tick'],'duration_ticks':f['duration_ticks']} for f in source['frames']],'material_authority':{'source_phase':2,'source_image':anchors[2]['source']['source'],'source_sha256':anchors[2]['source']['sha256'],'fixed_pattern_image':str(BASE/'rig-v1/phase-02-own-source-fill.png'),'fixed_pattern_sha256':hashlib.sha256((BASE/'rig-v1/phase-02-own-source-fill.png').read_bytes()).hexdigest(),'canonical_pose_parameters':anchors[2]['parameters'],'canonical_source_center':anchors[2]['source_center'],'canonical_source_bbox':anchors[2]['source']['bbox'],'rule':'One fixed anatomical UV map from own open phase2; source pattern cannot slide with later projected frames. Same image on inferred reverse.'},'constraints':['Conserved shared body/left/right wing geometry. Rigid body orientation and hinge rotations only.','All99 native phases are held for their exact two-tick duration; loop198ticks.','Hidden roll prior and smoothed body orientation are inferred, not measured flight attitude.','Own phase2 fixed UV/pattern preserves anatomical registration, but later source colors and silhouettes are approximate, reported rather than claimed exact.','Provisional altitude60 and source-derived screen path require separate full-map contact/occlusion review. No other butterfly propagated.']}
(OUT/'fit.json').write_text(json.dumps(packet,indent=2)+'\n');summary={'frames':99,'cycle_ticks':198,'source_centers':sum(r['source_centers'] for r in rows),'covered':sum(r['covered_source_centers'] for r in rows),'missed':sum(r['missing_source_centers'] for r in rows),'extra':sum(r['extra_centers'] for r in rows),'hinge_range':array[:,3:5].min(axis=0).tolist()+array[:,3:5].max(axis=0).tolist(),'loop_local_pose_exact':bool(np.array_equal(array[0],array[98])),'max_body_step_deg':float(abs(np.roll(array[:,:3],-1,axis=0)-array[:,:3]).max()),'max_hinge_step_deg':float(abs(np.roll(array[:,3:5],-1,axis=0)-array[:,3:5]).max())};(OUT/'fit-summary.json').write_text(json.dumps(summary,indent=2)+'\n');print(summary)
