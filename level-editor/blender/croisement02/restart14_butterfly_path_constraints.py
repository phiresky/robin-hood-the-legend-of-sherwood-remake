"""CPU-only flight-depth hypotheses with explicit source-order and canopy conflicts."""
import json,hashlib,math
from pathlib import Path
import numpy as np
from scipy.optimize import minimize,LinearConstraint,Bounds
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];W=ROOT/'level-editor/work/croisement02-refinement';B=W/'restart14-butterflies';O=B/'flight-path-constraints-v1';O.mkdir(exist_ok=True)
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest();depth=json.loads((B/'all7-depth-audit-v2/report.json').read_text());plan=json.loads((B/'all7-context-plan-v1/plan.json').read_text());sourcepath=W/'baseline/Croisement02.rhp.json';source=json.loads(sourcepath.read_text());layerspath=W/'source-states/layers.json';layers=json.loads(layerspath.read_text());sin=math.sin(math.radians(35));cos=math.cos(math.radians(35));fit=json.loads((B/'fixed-light-trial-v1/fit.json').read_text());rows=[];D=np.roll(np.eye(99),-1,axis=1)-np.eye(99);DD=D@D;patches=[]
for patch in layers['mission_patches']:
 key=(patch['name'],tuple(patch['state']['element_fx']['sprite'].values()))
 # Keep mission provenance while deduplicating identical resource frames by hash below.
 for state,info in patch.get('states',{}).items():
  for frame_index,f in enumerate(info.get('frames',[])):
   if f.get('image')and f.get('bbox'):patches.append((patch,state,frame_index,f))
cache={}
def smooth(lower):
 lower=np.array(lower);n=len(lower);dist=np.minimum(np.abs(np.arange(n)[:,None]-np.arange(n)),n-np.abs(np.arange(n)[:,None]-np.arange(n)));start=np.max(lower[None,:]-3*dist,axis=1)
 def obj(x):return float(np.mean(x)+.3*np.sum((DD@x)**2))
 def jac(x):return np.ones(n)/n+.6*DD.T@(DD@x)
 result=minimize(obj,start,jac=jac,method='SLSQP',bounds=Bounds(lower,np.full(n,400.)),constraints=[LinearConstraint(D,-3.,3.)],options={'maxiter':400,'ftol':1e-8});assert result.success,result.message;return result.x
sheet=Image.new('RGB',(1120,840),'#252525');draw=ImageDraw.Draw(sheet)
for j,s in enumerate(plan['sequences']):
 seq=s['index'];a=source['animations'][seq];assert a['display_polyline']==[]and a['blit_type']==0;rr=[r for r in depth['rays']if r['sequence']==seq];assert len(rr)==99
 for f in s['path']:assert sha(Path(f['source']))==f['sha256']
 center=np.array([r['screen']for r in rr]);body=center.copy()
 if seq==8:
  body=np.array([[p['source']['bbox'][0]+p['source_center'][0]+p['parameters'][5],p['source']['bbox'][1]+p['source_center'][1]+p['parameters'][6]]for p in fit['poses']])
 # Center-ray visibility is deliberately not treated as a measured flight altitude.
 heights=np.array([r['first_hit']['world_yup'][1]if r['first_hit']else 0 for r in rr]);canopy=np.array([bool(r['first_hit']and r['first_hit']['asset'].startswith('croisement02-tree-'))for r in rr]);lower=np.maximum(10,np.where(canopy,0,heights)+8);all_lower=np.maximum(10,heights+8);local=smooth(lower);visible=smooth(all_lower);state_overlaps={}
 for patch,state,fi,f in patches:
  x,y,w,h=f['bbox'];eligible=np.flatnonzero((center[:,0]>=x)&(center[:,0]<x+w)&(center[:,1]>=y)&(center[:,1]<y+h))
  if not len(eligible):continue
  path=W/'source-states'/f['image']
  if path not in cache:cache[path]=np.array(Image.open(path).convert('RGBA'))[:,:,3]
  alpha=cache[path];hit=[int(k)for k in eligible if alpha[int(center[k,1]-y),int(center[k,0]-x)]>0]
  if not hit:continue
  key=(patch['name'],state);entry=state_overlaps.setdefault(key,{'profile':patch['name'],'state':state,'missions':set(),'butterfly_phases':set(),'patch_frames':set(),'elevation':patch['state']['element_fx']['sprite']['elevation'],'integrate_in_background':patch['state']['integrate_in_background']});entry['missions'].add(patch['mission']);entry['butterfly_phases'].update(hit);entry['patch_frames'].add(fi)
 overlaps=[{k:sorted(v)if isinstance(v,set)else v for k,v in r.items()}for r in state_overlaps.values()]
 def describe(h):
  world=np.c_[body[:,0],-(body[:,1]+cos*h)/sin,h];err=np.max(abs(np.c_[world[:,0],-sin*world[:,1]-cos*world[:,2]]-body));return {'height_minmax':[float(h.min()),float(h.max())],'max_height_step':float(np.max(abs(D@h))),'max_world_step_per2ticks':float(np.linalg.norm(np.roll(world,-1,axis=0)-world,axis=1).max()),'native_projection_error':float(err),'knots_world_zup':world.tolist()}
 rows.append({'sequence':seq,'name':s['profile'],'source_behavior':a,'anchor_kind':'frozen body anchor'if seq==8 else'observed alpha-centroid proxy, not body registration','source_centroids':center.tolist(),'body_anchors':body.tolist(),'center_to_body_max_delta':float(np.linalg.norm(body-center,axis=1).max()),'firsthit_canopy_conflicts':[{'phase':r['phase'],'screen':r['screen'],'owner':r['first_hit']['asset'],'node':r['first_hit']['node'],'height':r['first_hit']['world_yup'][1]}for r,c in zip(rr,canopy)if c],'local_clearance_candidate':describe(local),'all_firsthit_visible_counterfactual':describe(visible),'state_source_overlaps':overlaps,'not_resolved':'Local path excludes crown constraints but underlying receivers and swept-wing collision are not yet sampled; counterfactual is not selected.'})
 y=j*120;draw.text((8,y+4),f'Butterfly{j+1}: cyan local-clearance hypothesis / orange all-visible counterfactual',fill='white')
 for h,color in [(local,'#63dccc'),(visible,'#ffad59')]:draw.line([(80+i*10,y+105-v*.25)for i,v in enumerate(h)],fill=color,width=2)
 for i in np.flatnonzero(canopy):draw.ellipse((78+i*10,y+101,82+i*10,y+105),fill='#ff6666')
 draw.text((8,y+27),f'{local.min():.0f}-{local.max():.0f}Z\n{visible.min():.0f}-{visible.max():.0f}Z',fill='white')
sheet.save(O/'seven-path-height-hypotheses.png');report={'status':'PLANNING_NOT_PHYSICAL_PATH_APPROVAL','source_sha256':sha(sourcepath),'mission_layers_sha256':sha(layerspath),'depth_report_sha256':sha(B/'all7-depth-audit-v2/report.json'),'fixed_model_sha256':sha(B/'fixed-light-trial-v1/model.blend'),'source_frames_verified':693,'all_clocks':'Independent99phase198tick streams preserved; no phase lock or shifted timing introduced.','constraint_method':'Minimum-height cyclic quadratic smooth envelope with max3Zunits/2ticks and8unit center clearance, both inferred choices. Not native speed/altitude evidence.','source_order_correction':'All seven display polylines are EMPTY. Elevated FX has empty-line mapY fallback, no measured masking contour. All blit_type0; ordinary sprite masking off.05 background capture/restore remains separate.','rows':rows,'next_bounded_proof':['Resolve vertex alpha/culling and source-vs-inferred canopy provenance at22conflicting center rays; no canopy edits.','Sample body-anchor and complete wing footprint, not centroid alone; retain allfirsthit layers to find lower receivers for canopy-conflict paths.','Review local/above-crown alternatives in three static cluster contexts plus05 phase ordering; do not choose global above-canopy lift.','Preserve mission hiding/scatter and endpoint precedence where source overlap records apply; state transitions are not permission to move physical receivers.','Only after context review apply selected path to one representative per cluster; no automatic7model propagation.'],'limitations':['Nearest texture alpha and static installed scene only; state overlaps are possible co-occurrence unions, not synchronized phase pairs or final geometric occlusion.','693anchors do not establish collision-free intervals. Source frames are stepped; interpolation between knots is an unapproved physical-motion hypothesis.','Source01 body anchor differs from centroid; depth samples must be repeated at the body/wing footprint before final selection.']};(O/'proposal.json').write_text(json.dumps(report,indent=2)+'\n');assert sum(p.stat().st_size for p in O.iterdir())<=2*1024**2;print([(r['name'],r['local_clearance_candidate']['height_minmax'],r['all_firsthit_visible_counterfactual']['height_minmax'],len(r['firsthit_canopy_conflicts']),len(r['state_source_overlaps']))for r in rows])
