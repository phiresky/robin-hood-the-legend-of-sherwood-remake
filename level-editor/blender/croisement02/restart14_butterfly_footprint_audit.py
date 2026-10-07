"""Read-only complete observed footprints and representative conserved-rig envelope."""
from pathlib import Path
import json,hashlib,math,sys,collections
import numpy as np
from PIL import Image
from scipy.spatial.transform import Rotation
from scipy.spatial import ConvexHull
from matplotlib.path import Path as Polygon
import restart14_butterfly_canopy22_audit as reader
ROOT=Path(__file__).resolve().parents[3];B=ROOT/'level-editor/work/croisement02-refinement/restart14-butterflies';O=B/'all7-footprint-audit-v2';O.mkdir(exist_ok=True)
plan=json.loads((B/'all7-context-plan-v1/plan.json').read_text());paths=json.loads((B/'flight-path-constraints-v1/proposal.json').read_text());fits=json.loads((B/'fixed-light-trial-v1/fit.json').read_text());wing=np.array(fits['wing_outline']);SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35));basis=np.array([[1,0,0],[0,-SIN,-COS],[0,-COS,SIN]]);body=np.array([[.36*math.sin(a)*math.cos(t),4.5*math.cos(a),.55*math.sin(a)*math.sin(t)]for a in np.linspace(0,math.pi,13)for t in np.linspace(0,math.tau,25)[:-1]]);rays=[];lookup={};frames=[]
def site(xy):
 key=tuple(float(v)for v in xy)
 if key not in lookup:lookup[key]=len(rays);rays.append({'screen':list(key),'hits':[]})
 return lookup[key]
for s,path in zip(plan['sequences'],paths['rows']):
 for f in s['path']:
  phase=f['phase'];a=np.array(Image.open(f['source']).convert('RGBA'));ys,xs=np.nonzero(a[:,:,3]);observed={site((f['bbox'][0]+x+.5,f['bbox'][1]+y+.5))for x,y in zip(xs,ys)};envelope=set(observed);zextent=8.;anchor=path['body_anchors'][phase];h=path['local_clearance_candidate']['knots_world_zup'][phase][2]
  if s['index']==8:
   p=np.array(fits['poses'][phase]['parameters']);g=Rotation.from_euler('xyz',p[:3],degrees=True).as_matrix();polys=[];verts=[body@g.T]
   for sign,angle in [(-1,p[3]),(1,p[4])]:
    v=wing.copy();v[:,0]*=sign;v=Rotation.from_euler('y',-sign*angle,degrees=True).apply(v);v[:,0]+=sign*.3;verts.append(v@g.T);polys.append((v@g.T)[:,:2]+anchor)
   v=verts[0][:,:2]+anchor;polys.append(v[ConvexHull(v).vertices]);allxy=np.vstack(polys);lo=np.floor(allxy.min(0)).astype(int);hi=np.ceil(allxy.max(0)).astype(int);gy,gx=np.mgrid[lo[1]:hi[1]+1,lo[0]:hi[0]+1];q=np.c_[gx.ravel()+.5,gy.ravel()+.5];inside=np.zeros(len(q),bool)
   for poly in polys:inside|=Polygon(poly).contains_points(q)
   envelope|={site(v)for v in q[inside]};zextent=float(np.max(abs((np.vstack(verts)@basis.T)[:,2]))+.085)
  anchorid=site(anchor);envelope.add(anchorid);frames.append({'sequence':s['index'],'phase':phase,'observed':sorted(observed),'envelope':sorted(envelope),'anchor':anchorid,'height':h,'z_extent':zextent,'physical_anatomy_checked':s['index']==8})
print('UNIQUE_RAYS',len(rays),'FRAMES',len(frames),flush=True)
def finish(rays,records,mp):
 summaries=[];examples=[]
 for f in frames:
  owners=collections.Counter();above=collections.Counter();intersect=collections.Counter();below=collections.Counter();source_above=0;lower_missing=0;minlower=None;maxupper=None;noncanopy_peak=0.;tree_intervals={}
  for ri in f['envelope']:
   hits=[h for h in rays[ri]['hits']if h['passes_alpha_only']];
   for h in hits:
    if h['asset'].startswith('croisement02-tree-'):tree_intervals.setdefault(h['asset'],[]).append(h['world_yup'][1])
    else:noncanopy_peak=max(noncanopy_peak,h['world_yup'][1])
   hits.sort(key=lambda h:h['camera_depth'],reverse=True);lower=[h for h in hits if h['world_yup'][1]<f['height']-f['z_extent']];middle=[h for h in hits if f['height']-f['z_extent']<=h['world_yup'][1]<=f['height']+f['z_extent']];upper=[h for h in hits if h['world_yup'][1]>f['height']+f['z_extent']]
   if hits:owners[hits[0]['asset']]+=1
   if not lower:lower_missing+=1
   else:
    nearest=max(lower,key=lambda h:h['world_yup'][1]);below[nearest['asset']]+=1;margin=f['height']-f['z_extent']-nearest['world_yup'][1];minlower=margin if minlower is None else min(minlower,margin)
   if middle:
    for asset in {h['asset']for h in middle}:intersect[asset]+=1
   if upper:
    for asset in {h['asset']for h in upper}:above[asset]+=1
    source_above+=ri in f['observed'];peak=max(h['world_yup'][1]for h in upper);maxupper=peak if maxupper is None else max(maxupper,peak)
   if (middle or upper) and len(examples)<120:
    examples.append({'sequence':f['sequence'],'phase':f['phase'],'screen':rays[ri]['screen'],'local_height':f['height'],'z_extent':f['z_extent'],'observed_source':ri in f['observed'],'first_valid':hits[0]if hits else None,'nearest_lower':max(lower,key=lambda h:h['world_yup'][1])if lower else None})
  summaries.append({k:f[k]for k in ['sequence','phase','height','z_extent','physical_anatomy_checked']}|{'observed_pixels':len(f['observed']),'tested_footprint_samples':len(f['envelope']),'first_hit_owners':dict(owners),'above_envelope_owners':dict(above),'possible_intersection_owners':dict(intersect),'nearest_lower_owners':dict(below),'observed_samples_with_upper_geometry':source_above,'samples_without_lower_receiver':lower_missing,'minimum_lower_clearance':minlower,'highest_upper_receiver':maxupper,'maximum_noncanopy_receiver_height':noncanopy_peak,'tree_height_intervals':{k:[min(v),max(v)]for k,v in tree_intervals.items()}})
 report={'status':'CPU_FOOTPRINT_CONSTRAINTS_NOT_PATH_APPROVAL','map_sha256':reader.sha(mp),'source_frames_verified':693,'unique_rays':len(rays),'tested_phase_footprint_samples':sum(len(f['envelope'])for f in frames),'phase_records':summaries,'examples':examples,'assets':records,'source_scope':'All positive sourceRGBA centers across7×99.01 additionally sampled conserved body+wing projected envelope. Other6 use observed footprint plus conservative8Zextent, no unapproved model propagation.','opacity':'All physical surfaces passing runtime level0bilinear alpha considered, including backs; rendering front-face culling remains separate. Foliage vertexalpha ignored per current shader. Mip filtering viewdependent remains outside this CPUtest.','collision_scope':'Vertical/depth envelope intersections conservatively flag possible contact, not exact triangle-mesh collision. Swept intervals and six unbuilt anatomies remain unproved.','source05':'Separate background capture/restore order retained; no static underlay or canopyownership transfer.','disk_policy_sha256':reader.sha(reader.W/'restart17-small-job-disk-policy.json')};(O/'report.json').write_text(json.dumps(report,indent=2)+'\n');assert sum(p.stat().st_size for p in O.iterdir())<8*1024**2;print('FOOTPRINT_COMPLETE',len(rays),sum(len(f['envelope'])for f in frames),flush=True)
 return report
reader.main(ray_records=rays,postprocess=finish,output=O)
