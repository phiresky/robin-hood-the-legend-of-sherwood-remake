"""CPU-only cyclic pose selection for conserved butterfly anatomy.

A finite multi-seed candidate bank is optimized jointly with quaternion
continuity. It is not a global continuous optimum or saved-model approval.
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
from restart14_butterfly_landmark_fit import LANDMARKS
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/croisement02-refinement/restart14-butterflies';OUT=BASE/'joint99-cpu-v1'
ANCHORS={**LANDMARKS,
0:{'tips':[[1.5,2.],[6.5,1.]],'center':[4.5,5.5],'axis':[0,1]},
2:{'tips':[[1.5,2.5],[10.5,1.5]],'center':[6.5,5.5],'axis':[0,1]},
8:{'axis':[0,1]},13:{'axis':[.6,.8]},
23:{'tips':[[1.5,1.5],[8.5,2.5]],'center':[6,4.5],'axis':[.1,1]},
37:{'axis':[-.55,.83]},42:{'axis':[.25,.97]},
47:{'tips':[[2.5,3.5],[8.5,3.5]],'center':[5.5,6.5],'axis':[0,1]},
53:{'axis':[0,1]},58:{'axis':[.1,1]},63:{'axis':[-.45,.89]},68:{'axis':[-.1,1]},
73:{'tips':[[3,2],[12,3]],'center':[8,6],'axis':[0,1]},
75:{'tips':[[3,2],[12,2]],'center':[7,6],'axis':[0,1]},
83:{'axis':[.6,.8]},88:{'axis':[.15,.99]},
93:{'tips':[[2,2],[11,1.5]],'center':[6.5,8],'axis':[0,1]},
}
# The previously held asymmetric phase gets weaker correspondence weights.
ANCHORS[30]={**LANDMARKS[30],'confidence':.4};ANCHORS[98]=ANCHORS[0]

def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 OUT.mkdir(exist_ok=True);packet=json.loads((BASE/'rig-full-v2/fit.json').read_text());old1=json.loads((BASE/'rig-full-v1/fit.json').read_text());land=json.loads((BASE/'pose-landmarks-v1/landmark-fit-proposal.json').read_text());land={r['phase']:r['versions']['Landmark hypothesis']['parameters'] for r in land['rows']};wing=np.array(packet['wing_outline']);ti=int(np.argmin(wing[:,1]));hi=int(np.argmax(wing[:,1]));body=np.array([[.36*math.sin(a)*math.cos(t),4.5*math.cos(a),.55*math.sin(a)*math.sin(t)] for a in np.linspace(0,math.pi,13) for t in np.linspace(0,math.tau,25)[:-1]])
 def geometry(p):
  g=Rotation.from_euler('xyz',p[:3],degrees=True).as_matrix();ws=[]
  for sign,a in [(-1,p[3]),(1,p[4])]:
   v=wing.copy();v[:,0]*=sign;v=Rotation.from_euler('y',-sign*a,degrees=True).apply(v);v[:,0]+=sign*.3;ws.append(v@g.T)
  return body@g.T,ws,g[:,1]
 contexts=[]
 for row in packet['poses']:
  a=np.array(Image.open(row['source']['source']).convert('RGBA'));mask=a[:,:,3]>0;h,w=mask.shape;ys,xs=np.nonzero(mask);center=np.array([xs.mean()+.5,ys.mean()+.5]);gy,gx=np.mgrid[-4:h+4,-4:w+4];q=np.column_stack((gx.ravel()+.5,gy.ravel()+.5));target=np.zeros(gx.shape,bool);target[4:4+h,4:4+w]=mask;target=target.ravel();dist=distance_transform_edt(~target.reshape(gx.shape)).ravel();weights=np.ones(gx.shape);weights[4:4+h,4:4+w]=.6+np.max(a[:,:,:3],axis=2)/255;contexts.append((a,center,q,target,dist,weights.ravel()))
 def evaluate(phase,p):
  a,center,q,target,dist,weights=contexts[phase];b,ws,axis=geometry(p);shift=center+np.array(p[5:]);b=b[:,:2]+shift;ws=[v[:,:2]+shift for v in ws];pred=PolygonPath(b[ConvexHull(b).vertices]).contains_points(q)
  for v in ws:pred|=PolygonPath(v).contains_points(q)
  tips=np.array([v[ti] for v in ws]);hind=np.array([v[hi] for v in ws]);axis=axis[:2]/max(np.linalg.norm(axis[:2]),1e-8);anchor=0.;lm=ANCHORS.get(phase)
  if lm:
   if 'tips' in lm:anchor+=np.sum((tips-np.array(lm['tips']))**2)
   if 'center' in lm:anchor+=.7*np.sum((shift-lm['center'])**2)
   if 'tip_mean' in lm:anchor+=2*np.sum((tips.mean(axis=0)-lm['tip_mean'])**2)
   if 'hind_mean' in lm:anchor+=2*np.sum((hind.mean(axis=0)-lm['hind_mean'])**2)
   intended=np.array(lm['axis'],float);intended/=np.linalg.norm(intended);anchor+=8*np.sum((axis-intended)**2);anchor*=lm.get('confidence',1.)
  sil=float(weights[target&~pred].sum()+(1+.25*dist[pred&~target]).sum())
  return {'parameters':np.array(p).tolist(),'source_centers':int(target.sum()),'covered':int((pred&target).sum()),'missing':int((~pred&target).sum()),'extra':int((pred&~target).sum()),'silhouette_loss':sil/target.sum(),'anchor_loss':float(anchor/target.sum()),'unary':float((sil+2.5*anchor)/target.sum()),'body_quaternion':Rotation.from_euler('xyz',p[:3],degrees=True).as_quat().tolist(),'projected_body_axis':axis.tolist(),'projected_tips':tips.tolist()}
 def transition(a,b):
  angle=2*np.arccos(np.clip(abs(np.dot(a['body_quaternion'],b['body_quaternion'])),0,1));hinges=np.deg2rad(np.array(a['parameters'][3:5])-b['parameters'][3:5]);return float(angle*angle+.1*np.sum(hinges*hinges))
 banks=[];bankpath=OUT/'candidate-bank.json'
 if bankpath.exists():banks=json.loads(bankpath.read_text())['phases'];print('RESUMED_BANK',len(banks),flush=True)
 for phase in range(len(banks),98):
  candidates=[]
  def add(p,label):
   e=evaluate(phase,p);e['candidate']=label
   if not any(np.linalg.norm(np.array(c['parameters'])-p)<1e-7 for c in candidates):candidates.append(e)
  add(packet['poses'][phase]['parameters'],'full-v2');add(old1['poses'][phase]['parameters'],'full-v1')
  if phase in land:add(land[phase],'landmark-proposal')
  for seed in [1703,2703]:
   def objective(p):return evaluate(phase,p)['unary']+.002*(p[3]-p[4])**2/contexts[phase][3].sum()
   fit=differential_evolution(objective,[(-85,85),(-85,85),(-100,100),(-88,88),(-88,88),(-4,4),(-4,4)],seed=seed+phase,popsize=6,maxiter=32,polish=False,tol=.008);add(fit.x,f'global-{seed+phase}')
  for c in list(candidates):
   p=np.array(c['parameters']);p[[0,1,3,4]]*=-1;add(p,c['candidate']+'-depth-alternative')
  banks.append(candidates);bankpath.write_text(json.dumps({'status':'PRIVATE_CPU_CANDIDATES','phases':banks},separators=(',',':'))+'\n')
  if phase%10==0:print('CANDIDATES',phase,'best',max(c['covered'] for c in candidates),flush=True)
 if len(banks)==98:
  banks.append([{**evaluate(98,c['parameters']),'candidate':c['candidate']} for c in banks[0]])
 def solve(weight):
  best=None
  for start in range(len(banks[0])):
   cost=np.full(len(banks[0]),np.inf);cost[start]=banks[0][start]['unary'];back=[]
   for phase in range(1,99):
    t=np.array([[transition(a,b) for b in banks[phase]] for a in banks[phase-1]])*weight;total=cost[:,None]+t;parent=np.argmin(total,axis=0);cost=total[parent,np.arange(len(parent))]+np.array([c['unary'] for c in banks[phase]]);back.append(parent)
   # Last source phase repeats the local first pose; exact loop is enforced.
   total=float(cost[start]);path=[start]
   for parent in reversed(back):path.append(int(parent[path[-1]]))
   path.reverse()
   if best is None or total<best[0]:best=(total,path)
  rows=[banks[i][k] for i,k in enumerate(best[1])];angles=[math.degrees(2*math.acos(min(1,abs(np.dot(rows[i]['body_quaternion'],rows[(i+1)%99]['body_quaternion']))))) for i in range(99)];return {'continuity_weight':weight,'objective':best[0],'rows':rows,'covered':sum(r['covered'] for r in rows),'missing':sum(r['missing'] for r in rows),'extra':sum(r['extra'] for r in rows),'silhouette_loss_sum':sum(r['silhouette_loss'] for r in rows),'anchor_loss_sum':sum(r['anchor_loss'] for r in rows),'continuity_cost':sum(transition(rows[i],rows[(i+1)%99]) for i in range(99)),'max_body_step_degrees':max(angles),'mean_body_step_degrees':float(np.mean(angles)),'per_phase_body_step_degrees':angles,'loop_pose_exact':rows[0]['parameters']==rows[98]['parameters']}
 solutions=[solve(w) for w in [0.,.1,.4,1.2]];selected=solutions[2];baseline=[evaluate(i,r['parameters']) for i,r in enumerate(packet['poses'])]
 # Fixed anatomy/pattern proposal only: report every phase including regressions.
 for i,row in enumerate(selected['rows']):row['phase']=i;row['baseline']=baseline[i];row['coverage_delta']=row['covered']-baseline[i]['covered']
 sheet=Image.new('RGB',(2640,1188),'#252525');d=ImageDraw.Draw(sheet)
 for i,row in enumerate(selected['rows']):
  a,center,*_=contexts[i];im=Image.fromarray(a);x=i%11*240;y=i//11*132;d.text((x+2,y+2),f"{i:02d} {row['covered']}/{row['source_centers']} ({row['coverage_delta']:+})",fill='white')
  for col,r in enumerate([baseline[i],row]):
   ox=x+col*120+10;oy=y+24;scale=5;img=im.resize((im.width*scale,im.height*scale),Image.Resampling.NEAREST);sheet.paste(img,(ox,oy),img);b,ws,_=geometry(r['parameters']);shift=center+np.array(r['parameters'][5:]);point=lambda v:(ox+v[0]*scale,oy+v[1]*scale)
   for poly,color in zip(ws,['#00ddff','#ff66ff']):poly=poly[:,:2]+shift;d.line([point(v) for v in np.vstack((poly,poly[0]))],fill=color,width=1)
   body2=b[:,:2]+shift;hull=body2[ConvexHull(body2).vertices];d.line([point(v) for v in np.vstack((hull,hull[0]))],fill='#ff9900',width=1)
 sheet.save(OUT/'all99-baseline-joint-proposal.png')
 report={'status':'CPU_ONLY_FINITE_CANDIDATE_JOINT_PROPOSAL','parent_model_sha256':sha(BASE/'rig-full-v2/model.blend'),'source_fit_sha256':sha(BASE/'rig-full-v2/fit.json'),'method':'Two independent global differential-evolution seeds per phase, prior fits, and valid opposite-depth pose candidates; exact cyclic dynamic programming over finite candidate bank. Quaternion geodesic body continuity, weaker hinge continuity. No claim of continuous global optimum.','anchors':ANCHORS,'manual_anchor_uncertainty_pixels':1.,'phase30_anchor_confidence':.4,'fixed_rest_geometry':packet['body_radii'],'fixed_material_authority':packet['material_authority'],'native_timing_unchanged':packet['source_clock'],'selected':selected,'tradeoffs':[{k:v for k,v in s.items() if k!='rows'} for s in solutions],'baseline_totals':{'covered':sum(r['covered'] for r in baseline),'missing':sum(r['missing'] for r in baseline),'extra':sum(r['extra'] for r in baseline),'anchor_loss_sum':sum(r['anchor_loss'] for r in baseline)},'limitations':['CPU analytical projection only; no model save or render.','Sparse manual source anatomical anchors include uncertain orientation-only hypotheses.','Phase30 remains explicitly uncertain; increased continuity does not prove its correspondence.','Source pixels, rest anatomy, fixed UV/pattern and99phase198tick clock unchanged.','No other six sequences propagated.']}
 (OUT/'joint-fit-proposal.json').write_text(json.dumps(report,indent=2)+'\n');bankpath.write_text(json.dumps({'status':'PRIVATE_CPU_CANDIDATES','phases':banks},separators=(',',':'))+'\n');print('JOINT_READY',[{k:s[k] for k in ['continuity_weight','covered','extra','max_body_step_degrees','anchor_loss_sum']} for s in solutions],flush=True)
if __name__=='__main__':main()
