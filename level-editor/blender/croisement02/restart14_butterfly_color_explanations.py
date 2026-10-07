"""Compare bounded side-pattern and static-lighting explanations read-only."""
from pathlib import Path
import json,hashlib
import numpy as np
from scipy.optimize import least_squares
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];OUT=ROOT/'level-editor/work/croisement02-refinement/restart14-butterflies/side-color-audit-v1'
def main():
 report=json.loads((OUT/'report.json').read_text());a=np.array(json.loads((OUT/'samples.json').read_text()),float);phase_data=report['phase_white_ratio'];phases=sorted(map(int,phase_data));target=np.array([phase_data[str(k)]['ratio'] for k in phases]);interior=np.array([phase_data[str(k)]['interior_fraction'] for k in phases]);normals=[a[(a[:,0]==k)&(a[:,17]>0),14:17] for k in phases];models={}
 def prediction(params,filtered):
  values=np.array([params[0]+np.maximum(n@params[1:4],0).mean() for n in normals])
  return values*(params[4]+(1-params[4])*interior) if filtered else values
 for name,filtered in [('nonnegative_ambient_lambert',False),('lambert_with_edge_fraction_proxy',True)]:
  lower=[0,-3,-3,-3]+([0] if filtered else []);upper=[2,3,3,3]+([1] if filtered else []);initial=[.1,0,0,.8]+([.6] if filtered else []);fit=least_squares(lambda p:prediction(p,filtered)-target,initial,bounds=(lower,upper),loss='soft_l1',f_scale=.15,max_nfev=1000);values=prediction(fit.x,filtered);held=np.zeros(len(target))
  for fold in range(3):
   mask=np.array([(k//11)%3==fold for k in phases]);f=least_squares(lambda p:(prediction(p,filtered)-target)[~mask],initial,bounds=(lower,upper),loss='soft_l1',f_scale=.15,max_nfev=1000);held[mask]=prediction(f.x,filtered)[mask]
  models[name]={'parameters':fit.x.tolist(),'parameter_names':['ambient','light_x','light_y','light_z']+(['edge_floor_fraction'] if filtered else []),'training_r_squared':float(1-np.sum((target-values)**2)/np.sum((target-target.mean())**2)),'held_block_mae':float(np.mean(abs(target-held))),'predictions':dict(zip(map(str,phases),values.tolist())),'physical_scope':'One fixed positive-ambient/direct-light hypothesis in canonical source-camera coordinates, not a recovered or approved map light.'}
 # Compare side-specific versus shared anatomical texel medians on held phases.
 # No texture is emitted; unseen training bins are explicitly counted.
 comparisons={}
 for name,side_specific in [('shared_side_uv_median',False),('separate_side_uv_median',True)]:
  errors=[];lower_errors=[];missing=0
  for fold in range(3):
   test=np.array([(int(k)//11)%3==fold for k in a[:,0]]);train=a[~test];lookup={}
   for row in train:
    key=(int(row[3]),int(row[12]),int(row[13]))+((int(row[4]),) if side_specific else ());lookup.setdefault(key,[]).append(row[6:9])
   lookup={k:np.median(v,axis=0) for k,v in lookup.items()}
   for row in a[test]:
    if row[5]<=1.01:continue
    key=(int(row[3]),int(row[12]),int(row[13]))+((int(row[4]),) if side_specific else ())
    if key not in lookup:missing+=1;continue
    error=float(np.mean(abs(row[6:9]-lookup[key])));errors.append(error)
    if row[4]==1:lower_errors.append(error)
  comparisons[name]={'interior_observations_compared':len(errors),'missing_training_bin':missing,'held_block_rgb_mae':float(np.mean(errors)),'lower_observations_compared':len(lower_errors),'lower_held_block_rgb_mae':float(np.mean(lower_errors))}
 paired=[]
 for fold in range(3):
  test=np.array([(int(k)//11)%3==fold for k in a[:,0]]);lookups=[]
  for sided in [False,True]:
   groups={}
   for row in a[~test]:
    key=(int(row[3]),int(row[12]),int(row[13]))+((int(row[4]),) if sided else ());groups.setdefault(key,[]).append(row[6:9])
   lookups.append({key:np.median(values,axis=0) for key,values in groups.items()})
  for row in a[test]:
   if row[5]<=1.01:continue
   key=(int(row[3]),int(row[12]),int(row[13]));sidekey=key+(int(row[4]),)
   if key in lookups[0] and sidekey in lookups[1]:paired.append([int(row[0]),int(row[4]),float(np.mean(abs(row[6:9]-lookups[0][key]))),float(np.mean(abs(row[6:9]-lookups[1][sidekey])))])
 paired=np.array(paired);paired_summary={}
 for name,mask in [('all',np.ones(len(paired),bool)),('lower',paired[:,1]==1)]:
  values=paired[mask];paired_summary[name]={'same_samples':len(values),'shared_rgb_mae':float(values[:,2].mean()),'separate_rgb_mae':float(values[:,3].mean()),'improvement':float((values[:,2]-values[:,3]).mean())}
 both=[]
 for row in report['phases']:
  d=row['observed_color_by_conditional_side'];up=d['interior_white_upper'];lo=d['interior_white_lower']
  if up['count']>=3 and lo['count']>=3:both.append({'phase':row['phase'],'upper_y':up['mean_linear_luminance'],'lower_y':lo['mean_linear_luminance'],'lower_minus_upper':lo['mean_linear_luminance']-up['mean_linear_luminance']})
 evidence={'audit_sha256':hashlib.sha256((OUT/'report.json').read_bytes()).hexdigest(),'fixed_lighting_hypotheses':models,'held_phase_uv_pattern_comparison':comparisons,'paired_same_sample_uv_comparison':paired_summary,'both_sides_interior_white_comparison':both,'lower_brighter_count':sum(r['lower_minus_upper']>0 for r in both),'comparison_phase_count':len(both),'limitations':['Side labels depend on inferred poses and cannot independently recover source-side pigment.','Fixed phase2 reflectance is itself lit artwork, so fitting illumination on top can double-count baked light.','Edge-fraction factor is a diagnostic proxy, not an approved opacity/shading algorithm.','Shared/side UV medians are evaluated only; no new textures/materials were generated.']};(OUT/'explanation-comparison.json').write_text(json.dumps(evidence,indent=2)+'\n')
 im=Image.new('RGB',(1200,520),'#252525');d=ImageDraw.Draw(im);d.text((20,10),'Observed white-pattern intensity ratio vs fixed explanations (all99 phases; gaps have too few samples)',fill='white');x0,y0,ww,hh=55,50,1080,330
 for val in [0,.5,1,1.5]:y=y0+hh-val/1.5*hh;d.line((x0,y,x0+ww,y),fill='#444444');d.text((10,y-5),str(val),fill='white')
 series=[('observed source',dict(zip(map(str,phases),target.tolist())),'#ffffff')]+[(name,m['predictions'],color) for (name,m),color in zip(models.items(),['#00dddd','#ffaa00'])]
 for index,(name,data,color) in enumerate(series):d.line([(x0+k/98*ww,y0+hh-float(data[str(k)])/1.5*hh) for k in phases],fill=color,width=2);d.text((20,410+index*24),name,fill=color)
 for k in range(0,99,10):d.text((x0+k/98*ww,385),str(k),fill='white')
 d.text((20,495),'Conditional geometry + filtering remain confounded. This does not establish a new wing-side texture pair.',fill='white');im.save(OUT/'fixed-explanation-comparison.png')
 source=Image.open(OUT/'source-and-conditional-side.png');selected=Image.new('RGB',(1000,720),'#252525')
 for i,row in enumerate([0,5,8,10]):selected.paste(source.crop((0,row*180,1000,(row+1)*180)),(0,i*180))
 selected.save(OUT/'key-side-comparison.png');print(json.dumps({**{k:{q:v for q,v in m.items() if q!='predictions'} for k,m in models.items()},'uv':comparisons,'lower_brighter':evidence['lower_brighter_count'],'both_sides':len(both)},indent=2))
if __name__=='__main__':main()
