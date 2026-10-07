"""Classify saved motion support gaps without changing any model or source art."""
from pathlib import Path
from collections import Counter
import json,hashlib
import numpy as np
from PIL import Image,ImageDraw
from scipy.ndimage import distance_transform_edt,gaussian_filter,map_coordinates
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/croisement02-refinement/restart14-canopy-animation';WORK=BASE/'tree42-motion-v4';DEST=BASE/'tree42-gap-classification-v2';sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def sample(a,x,y):
 scalar=np.ndim(x)==0;x=np.atleast_1d(x);y=np.atleast_1d(y)
 if a.ndim==2:return map_coordinates(a,[y,x],order=1,mode='constant')
 v=np.stack([map_coordinates(a[:,:,i],[y,x],order=1,mode='constant')for i in range(a.shape[2])],axis=-1);return v[0]if scalar else v
def main():
 DEST.mkdir(exist_ok=True);assert not(DEST/'report.json').exists();source=json.loads((BASE/'source-reconciliation-v1/report.json').read_text())['groups'][1];images=[]
 for phase in (0,7):
  f=source['frames'][phase];x,y,w,h=f['bbox'];im=Image.new('RGBA',(342,288));im.paste(Image.open(f['path']),(x-616,y-688));images.append(im)
 images +=[Image.open(WORK/f'native-phases-v1/phase-{i:02}.png').convert('RGBA')for i in(0,7)];arrays=[np.array(im)for im in images];dist=[distance_transform_edt(a[:,:,3]<128)for a in arrays];features=[]
 for a in arrays[:2]:
  f=a.astype(float)/255;f[:,:,:3]*=f[:,:,3:];features.append(gaussian_filter(f,(2,2,0)))
 field=np.load(BASE/'tree42-coherent-correspondence-v2/flows.npz')['flow'][7];coords=[p['native_pixel']for p in json.loads((WORK/'native-phases-v1/phase7-distant-support-gaps.json').read_text())['pixels']]+[[862,775]];ray=json.loads((BASE/'tree42-alpha-neighbors-coherent-v1/report.json').read_text());misses=set(map(tuple,ray['phases'][7]['tree42_alpha_misses']));foreign={tuple(p['pixel']):p['asset']for p in ray['phases'][7]['foreign_first_hits']};baseline_solid=json.loads((BASE/'tree42-motion-v1/report.json').read_text())['phase0_solid_support'][7]['missing'];gy,gx=np.mgrid[-5:6,-5:6];weights=np.exp(-(gx*gx+gy*gy)/18);weights/=weights.sum();rows=[];sheets=[]
 for n,(sx,sy)in enumerate(coords):
  x,y=sx-616,sy-688;target=sample(features[1],x+gx,y+gy);q=np.array([x,y],float)
  for _ in range(16):q=np.array([x,y])-sample(field,np.array(q[0]),np.array(q[1]))
  predicted=np.array([x,y])-q
  cost=lambda d:float(np.sum(np.mean((sample(features[0],x+gx-d[0],y+gy-d[1])-target)**2,axis=2)*weights))
  candidates=sorted((cost((dx,dy)),dx,dy)for dy in np.arange(-4,4.01,.5)for dx in np.arange(-4,4.01,.5));best=np.array(candidates[0][1:]);second=next(c for c in candidates if np.linalg.norm(np.array(c[1:])-best)>=1.5);basegap=dist[2][y,x]>2;classification='unchanged_baseline_solid_miss'if [sx,sy]in baseline_solid else 'baseline_missing_support'if basegap else 'motion_introduced_distant_gap';class_extra='unchanged_baseline_solid_miss'if [sx,sy]in baseline_solid else 'baseline_solid_support_exists';delta=cost(predicted)-candidates[0][0];agreement=float(np.linalg.norm(best-predicted));confidence=second[0]/max(candidates[0][0],1e-12);action='test_local_cluster_correspondence_constraint'if delta/max(cost(predicted),1e-12)>.15 and confidence>1.25 and agreement>.35 and np.max(np.abs(best))<4 else 'retain_as_ambiguous_coverage_constraint_not_forced_pixel_motion'
  row={'native_pixel':[sx,sy],'classification':classification,'solid_classification':class_extra,'source0_opaque':bool(arrays[0][y,x,3]),'source7_opaque':bool(arrays[1][y,x,3]),'baseline_render_distance':float(dist[2][y,x]),'phase7_render_distance':float(dist[3][y,x]),'source0_distance':float(dist[0][y,x]),'phase7_alpha_ray_miss':(sx,sy)in misses,'foreign_receiver':foreign.get((sx,sy)),'inverse_predicted_source_point':[float(q[0]+616),float(q[1]+688)],'predicted_displacement':predicted.tolist(),'independent_lowpass_patch_best_displacement':best.tolist(),'patch_best_cost':candidates[0][0],'patch_predicted_cost':cost(predicted),'patch_stationary_cost':cost((0,0)),'patch_alternative_ratio':confidence,'correction_difference_pixels':agreement,'proposal':action};rows.append(row)
  if n%6==0:sheets.append(Image.new('RGB',(576,6*126+36),'#292929'));ImageDraw.Draw(sheets[-1]).text((8,8),'Own source0 | own source7 | static actual0 | moving actual7',fill='white')
  sheet=sheets[-1];draw=ImageDraw.Draw(sheet);top=36+(n%6)*126;draw.text((8,top),f'({sx},{sy}) base {dist[2][y,x]:.2f}px -> phase7 {dist[3][y,x]:.2f}px; {classification}',fill='white')
  for i,im in enumerate(images):
   crop=im.crop((x-9,y-9,x+10,y+10)).resize((95,95),Image.Resampling.NEAREST);left=i*144+22;sheet.paste(crop,(left,top+22),crop);draw.rectangle((left+45,top+67,left+49,top+71),outline='#ff5555')
 for i,sheet in enumerate(sheets):sheet.save(DEST/f'crops-{i+1}.png')
 report={'status':'READ_ONLY_GAP_CLASSIFICATION_FOR_CORRESPONDENCE_REFINEMENT','prototype_sha256':sha(WORK/'prototype.blend'),'rows':rows,'classification_counts':dict(Counter(r['classification']for r in rows[:21])),'proposal_counts':dict(Counter(r['proposal']for r in rows)),'source_evidence':[{'path':f['path'],'sha256':sha(Path(f['path']))}for f in(source['frames'][0],source['frames'][7])],'limits':['Baseline-missing means the stationary crown lacks support at a phase7 target coordinate; it is not an observed phase0 source defect.','Baseline versus motion classification uses saved complete-crown raster alpha, not a new geometry ray test.','The independently saved phase0 solid-ray audit explicitly establishes862,775 already misses stationary geometry when queried by phase7 source.','Local11x11 patches use sigma2 premultiplied own-source RGBA; half-pixel search proposals are candidates, not accepted exact leaf correspondences.','Nearby inferred rear coverage does not become observed-front ownership; all raw residuals remain.','No model, material, UV, source texture, neighbor, runtime or shared catalog was changed.']};(DEST/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(report['classification_counts']);print(report['proposal_counts']);print([(r['native_pixel'],r['baseline_render_distance'],r['independent_lowpass_patch_best_displacement'],round(r['patch_alternative_ratio'],2))for r in rows])
if __name__=='__main__':main()
