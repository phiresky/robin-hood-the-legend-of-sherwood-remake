"""Compare the one fixed-lighting trial against source and frozen native views."""
from pathlib import Path
import json,hashlib
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];B=ROOT/'level-editor/work/croisement02-refinement/restart14-butterflies';O=B/'fixed-light-trial-v1';sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def linear(a):
 a=a.astype(float)/255;return np.where(a<=.04045,a/12.92,((a+.055)/1.055)**2.4)
def measure(folder):
 f=json.loads((folder/'fit.json').read_text());rows=[]
 for r in f['poses']:
  phase=r['phase'];src=np.array(Image.open(r['source']['source']).convert('RGBA'));render=np.array(Image.open(folder/'motion'/f'phase-{phase:02d}-view-0.png').convert('RGBA'));ys,xs=np.nonzero(src[:,:,3]);center=np.array(r['source_center'])+r['parameters'][5:7];loc=np.floor((np.c_[xs+.5,ys+.5]-center)*8+96).astype(int);inside=np.all((loc>=0)&(loc<192),axis=1);samples=np.zeros((len(xs),4),np.uint8);samples[inside]=render[loc[inside,1],loc[inside,0]];hit=samples[:,3]>127;a=samples[hit,:3];b=src[ys[hit],xs[hit],:3];rows.append({'phase':phase,'source':len(xs),'covered':int(hit.sum()),'rgb_mae':float(np.abs(a.astype(float)-b).mean()),'linear_mae':float(np.abs(linear(a)-linear(b)).mean())})
 return {'rows':rows,'covered':sum(r['covered']for r in rows),'source':sum(r['source']for r in rows),'mean_phase_rgb_mae':float(np.mean([r['rgb_mae']for r in rows])),'mean_phase_linear_mae':float(np.mean([r['linear_mae']for r in rows])),'phase20_29_rgb_mae':float(np.mean([r['rgb_mae']for r in rows[20:30]]))}
a=measure(B/'rig-joint-trial-v1');b=measure(O);check=json.loads((O/'frame-checkpoints.json').read_text());assert len(check['completed'])==124 and all(sha(O/p)==h for p,h in check['completed'].items());report={'baseline':a,'fixed_light':b,'validation_sha256':sha(O/'validation.json'),'model_sha256':sha(O/'model.blend'),'all124_frame_hashes_verified':True,'status':'MEASURED_NOT_PIXEL_PARITY'};(O/'fixed-light-metrics.json').write_text(json.dumps(report,indent=2)+'\n');print({k:{j:v for j,v in r.items()if j!='rows'}for k,r in [('baseline',a),('fixed_light',b)]})
