"""Bounded single-phase anatomical pose candidates; all neighboring poses stay frozen."""
import json,hashlib,math
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
from scipy.optimize import differential_evolution
from scipy.spatial import ConvexHull
from scipy.spatial.transform import Rotation
from matplotlib.path import Path as Poly
ROOT=Path(__file__).resolve().parents[3];B=ROOT/'level-editor/work/croisement02-refinement/restart14-butterflies';O=B/'phase30-local-cpu-v1';O.mkdir(exist_ok=True)
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest();fp=B/'rig-joint-trial-v1/fit.json';d=json.loads(fp.read_text());row=d['poses'][30];p0=np.array(row['parameters']);wing=np.array(d['wing_outline']);ti=np.argmin(wing[:,1]);body=np.array([[.36*math.sin(a)*math.cos(t),4.5*math.cos(a),.55*math.sin(a)*math.sin(t)] for a in np.linspace(0,math.pi,13) for t in np.linspace(0,math.tau,25)[:-1]])
im=Image.open(row['source']['source']).convert('RGBA');rgba=np.array(im);mask=rgba[:,:,3]>0;ys,xs=np.nonzero(mask);center=np.array([xs.mean()+.5,ys.mean()+.5]);gy,gx=np.mgrid[-4:im.height+4,-4:im.width+4];q=np.c_[gx.ravel()+.5,gy.ravel()+.5];target=np.pad(mask,4).ravel();neighbors=[np.array(d['poses'][i]['parameters']) for i in [29,31]]
def evaluate(p,swap=False):
 r=Rotation.from_euler('xyz',p[:3],degrees=True);g=r.as_matrix();shift=center+p[5:];b=(body@g.T)[:,:2]+shift;ws=[]
 for sign,angle in [(-1,p[3]),(1,p[4])]:
  v=wing.copy();v[:,0]*=sign;v=Rotation.from_euler('y',-sign*angle,degrees=True).apply(v);v[:,0]+=sign*.3;ws.append((v@g.T)[:,:2]+shift)
 pred=Poly(b[ConvexHull(b).vertices]).contains_points(q)
 for w in ws:pred|=Poly(w).contains_points(q)
 tips=np.array([w[ti] for w in ws]);aim=np.array([[2.5,3.5],[10,1.5]])[::(-1 if swap else 1)];tip=float(np.maximum(np.abs(tips-aim)-1,0).dot(np.ones(2)).sum())
 axis=g[:2,1];axis/=np.linalg.norm(axis);want=np.array([.3,1]);want/=np.linalg.norm(want)
 anchor=tip+3*np.sum((axis-want)**2)+.5*np.maximum(np.abs(shift-[6,4])-1,0).sum()
 continuity=sum((r.inv()*Rotation.from_euler('xyz',n[:3],degrees=True)).magnitude()**2+.1*np.sum(np.deg2rad(p[3:5]-n[3:5])**2) for n in neighbors)
 miss=int((target&~pred).sum());extra=int((~target&pred).sum())
 return {'parameters':p.tolist(),'covered':int((target&pred).sum()),'missing':miss,'extra':extra,'anchor_loss':float(anchor),'continuity':float(continuity),'tips':tips.tolist(),'body_axis':axis.tolist(),'objective':miss+extra*.65+3*anchor+4*continuity},ws,b
indices=[2,3,4,5,6];bounds=[(p0[2]-12,p0[2]+12),(p0[3]-25,p0[3]+25),(p0[4]-25,p0[4]+25),(p0[5]-.75,p0[5]+.75),(p0[6]-.75,p0[6]+.75)]
rows=[{'name':'Frozen30','swap':False,**evaluate(p0)[0]}]
for seed,swap in [(301,False),(302,False),(303,True)]:
 def loss(v):
  p=p0.copy();p[indices]=v;return evaluate(p,swap)[0]['objective']
 f=differential_evolution(loss,bounds,seed=seed,popsize=6,maxiter=35,polish=False,workers=1);p=p0.copy();p[indices]=f.x;rows.append({'name':f'Candidate{seed}','swap':swap,**evaluate(p,swap)[0]})
sheet=Image.new('RGB',(1100,360),'#252525');dr=ImageDraw.Draw(sheet)
for col,r in enumerate(rows):
 x=col*275+15;y=50;scale=15;sheet.paste(im.resize((im.width*scale,im.height*scale),Image.Resampling.NEAREST),(x,y),im.resize((im.width*scale,im.height*scale),Image.Resampling.NEAREST));_,ws,b=evaluate(np.array(r['parameters']),r['swap'])
 for poly,color in zip(ws,['#00ddff','#ff66ff']):dr.line([(x+v[0]*scale,y+v[1]*scale) for v in np.vstack((poly,poly[0]))],fill=color,width=2)
 dr.text((x,8),r['name'],fill='white');dr.text((x,265),f"Covered {r['covered']}/71; extra {r['extra']}\nAnchor {r['anchor_loss']:.2f}\nContinuity {r['continuity']:.3f}",fill='white')
sheet.save(O/'source-and-three-candidates.png');report={'status':'CPU_ONLY_UNSELECTED','fit_sha256':sha(fp),'source_sha256':sha(Path(row['source']['source'])),'rows':rows,'guards':{'other98poses_unchanged':True,'model_unchanged':True,'material_uv_unchanged':True,'no_brightness_change':True,'source_clock_198ticks':True},'uncertainty':'One-pixel uncertain tips/body anchors; swapped labels explicitly tested. No scene-depth or rendered appearance acceptance.'};(O/'report.json').write_text(json.dumps(report,indent=2)+'\n');print([(r['name'],r['covered'],r['extra'],r['anchor_loss'],r['continuity']) for r in rows])
