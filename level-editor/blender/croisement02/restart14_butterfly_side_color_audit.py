"""Audit observed butterfly color against fixed-pose wing sides, without edits.

Side labels are conditional on the frozen anatomical hypothesis, not recovered
facts about the artwork. Geometry is reconstructed from its pinned recipe.
"""
from pathlib import Path
import hashlib,json,math
import numpy as np
from PIL import Image,ImageDraw
from scipy.ndimage import distance_transform_edt
from scipy.spatial.transform import Rotation
from scipy.optimize import least_squares
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/croisement02-refinement/restart14-butterflies';OUT=BASE/'side-color-audit-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def linear(rgb):
 a=np.asarray(rgb,float)/255;return np.where(a<=.04045,a/12.92,((a+.055)/1.055)**2.4)
def luminance(rgb):return linear(rgb)@np.array([.2126,.7152,.0722])
def main():
 OUT.mkdir(exist_ok=True);fitpath=BASE/'rig-joint-trial-v1/fit.json';fit=json.loads(fitpath.read_text());outline=np.array(fit['wing_outline']);count=len(outline);front=np.vstack(([2,-.3,.19],outline));back=front.copy();back[:,2]-=fit['wing_thickness'];wing=np.vstack((front,back));wf=[]
 for i in range(count):
  a=1+i;b=1+(i+1)%count;wf.extend([([0,a,b],'upper'),([count+1,count+1+b,count+1+a],'lower'),([a,count+1+a,count+1+b],'edge'),([a,count+1+b,b],'edge')])
 vertices=[[0,4.5,0]];faces=[];rings=12;segments=24
 for k in range(1,rings):
  angle=math.pi*k/rings
  for j in range(segments):t=math.tau*j/segments;vertices.append([.36*math.sin(angle)*math.cos(t),4.5*math.cos(angle),.55*math.sin(angle)*math.sin(t)])
 vertices.append([0,-4.5,0]);end=len(vertices)-1
 for j in range(segments):faces.extend([[0,1+(j+1)%segments,1+j],[end,1+(rings-2)*segments+j,1+(rings-2)*segments+(j+1)%segments]])
 for k in range(rings-2):
  for j in range(segments):a=1+k*segments+j;b=1+k*segments+(j+1)%segments;faces.extend([[a,b,b+segments],[a,b+segments,a+segments]])
 body=np.array(vertices);authority=fit['material_authority'];pattern=np.array(Image.open(authority['fixed_pattern_image']).convert('RGBA'));assert sha(Path(authority['fixed_pattern_image']))==authority['fixed_pattern_sha256'];canonical=authority['canonical_pose_parameters'];cg=Rotation.from_euler('xyz',canonical[:3],degrees=True).as_matrix();cshift=np.array(authority['canonical_source_center'])+canonical[5:];rows=[];samples=[];owner_images={};phase_groups={}
 for phase,row in enumerate(fit['poses']):
  source=Path(row['source']['source']);assert sha(source)==row['source']['sha256'];rgba=np.array(Image.open(source).convert('RGBA'));mask=rgba[:,:,3]>0;ys,xs=np.nonzero(mask);points=np.column_stack((xs+.5,ys+.5));params=np.array(row['parameters']);center=np.array(row['source_center'])+params[5:];q=points-center;rotation=Rotation.from_euler('xyz',params[:3],degrees=True).as_matrix();depth=np.full(len(q),-np.inf);owners=np.full(len(q),-1,int);side=np.full(len(q),-1,int);normals=np.zeros((len(q),3));local_hit=np.zeros((len(q),3))
  meshes=[]
  for owner,sign,angle in [(0,-1,params[3]),(1,1,params[4])]:
   local=wing.copy();local[:,0]*=sign;posed=Rotation.from_euler('y',-sign*angle,degrees=True).apply(local);posed[:,0]+=sign*.3;posed=posed@rotation.T;meshfaces=[(indices if sign==1 else list(reversed(indices)),tag) for indices,tag in wf];meshes.append((owner,local,posed,meshfaces))
  meshes.append((2,body,body@rotation.T,[(indices,'body') for indices in faces]))
  for owner,local,posed,meshfaces in meshes:
   for indices,tag in meshfaces:
    tri=posed[indices];a,b,c=tri[:,:2];v0=b-a;v1=c-a;den=v0[0]*v1[1]-v1[0]*v0[1]
    if abs(den)<1e-10:continue
    v2=q-a;u=(v2[:,0]*v1[1]-v1[0]*v2[:,1])/den;v=(v0[0]*v2[:,1]-v2[:,0]*v0[1])/den;inside=(u>=-1e-9)&(v>=-1e-9)&(u+v<=1+1e-9);z=tri[0,2]+u*(tri[1,2]-tri[0,2])+v*(tri[2,2]-tri[0,2]);take=inside&(z>depth+1e-8)
    if not np.any(take):continue
    weights=np.column_stack((1-u-v,u,v));normal=np.cross(tri[1]-tri[0],tri[2]-tri[0]);normal/=np.linalg.norm(normal);depth[take]=z[take];owners[take]=owner;side[take]={'upper':0,'lower':1,'edge':2,'body':3}[tag];normals[take]=normal;local_hit[take]=weights[take]@local[indices]
  distances=distance_transform_edt(mask)[ys,xs];colors=rgba[ys,xs,:3];observed=np.array(Image.open(BASE/'rig-joint-trial-v1/motion'/f'phase-{phase:02d}-view-0.png').convert('RGBA'));pixel=np.floor(q*8+96).astype(int);render_covered=observed[pixel[:,1],pixel[:,0],3]>127;ref=np.zeros((len(q),3),np.uint8);uv=np.full((len(q),2),-1,int)
  for owner,sign in [(0,-1),(1,1)]:
   sel=owners==owner;v=Rotation.from_euler('y',-sign*canonical[3+owner],degrees=True).apply(local_hit[sel]);v[:,0]+=sign*.3;xy=(v@cg.T)[:,:2]+cshift;ij=np.floor(xy).astype(int);ij[:,0]=np.clip(ij[:,0],0,pattern.shape[1]-1);ij[:,1]=np.clip(ij[:,1],0,pattern.shape[0]-1);ref[sel]=pattern[ij[:,1],ij[:,0],:3];uv[sel]=ij
  broad=(owners<2)&(owners>=0)&(side<2)&(np.abs(normals[:,2])>.15)&render_covered;white_ref=(luminance(ref)>.55)&(ref[:,2]>ref[:,1]*.65);analysis=broad&white_ref;interior=analysis&(distances>1.01);data={}
  for label,sel in [('upper',broad&(side==0)),('lower',broad&(side==1)),('white_upper',analysis&(side==0)),('white_lower',analysis&(side==1)),('interior_white_upper',interior&(side==0)),('interior_white_lower',interior&(side==1))]:
   data[label]={'count':int(sel.sum()),'mean_srgb':colors[sel].mean(axis=0).tolist() if sel.any() else None,'mean_linear_luminance':float(luminance(colors[sel]).mean()) if sel.any() else None}
  simple=[]
  for sign,angle in [(-1,params[3]),(1,params[4])]:simple.append(float((rotation@Rotation.from_euler('y',-sign*angle,degrees=True).apply([0,0,1]))[2]))
  for i in np.where(broad)[0]:samples.append([phase,int(xs[i]),int(ys[i]),int(owners[i]),int(side[i]),float(distances[i]),*colors[i].tolist(),*ref[i].tolist(),*uv[i].tolist(),*normals[i].tolist(),bool(white_ref[i])])
  if analysis.sum()>=4:
   phase_groups[phase]={'ratio':float(np.median(luminance(colors[analysis])/np.maximum(luminance(ref[analysis]),1e-4))),'lower_fraction':float(np.mean(side[analysis]==1)),'normal':normals[analysis].mean(axis=0).tolist(),'interior_fraction':float(np.mean(distances[analysis]>1.01)),'sample_count':int(analysis.sum())}
  rowreport={'phase':phase,'source_sha256':row['source']['sha256'],'source_positive_pixels':len(xs),'cpu_first_hit_count':int((owners>=0).sum()),'render_hit_count':int(render_covered.sum()),'cpu_render_coverage_disagreement':int(np.sum((owners>=0)!=render_covered)),'body_count':int(np.sum(owners==2)),'edge_count':int(np.sum(side==2)),'simple_signed_wing_cosines':simple,'observed_color_by_conditional_side':data};rows.append(rowreport)
  display=np.zeros_like(rgba);display[ys,xs,:3]=np.array([([20,180,255] if s==0 else [255,140,40] if s==1 else [180,180,180] if s>=2 else [140,30,160]) for s in side],np.uint8);display[ys,xs,3]=255;owner_images[phase]=Image.fromarray(display)
 # Phase-held-out comparisons are descriptive only: pose and source filtering
 # remain confounded, and they are not a recovered lighting rig.
 keys=sorted(phase_groups);y=np.array([phase_groups[k]['ratio'] for k in keys]);norm=np.array([phase_groups[k]['normal'] for k in keys]);lower=np.array([phase_groups[k]['lower_fraction'] for k in keys]);edge=np.array([phase_groups[k]['interior_fraction'] for k in keys]);models={}
 for name,x in [('constant',np.ones((len(keys),1))),('side_fraction',np.column_stack((np.ones(len(keys)),lower))),('normal_affine',np.column_stack((np.ones(len(keys)),norm))),('normal_plus_interior',np.column_stack((np.ones(len(keys)),norm,edge)))]:
  coeff=np.linalg.lstsq(x,y,rcond=None)[0];pred=x@coeff;held=np.zeros(len(y))
  for fold in range(3):
   test=np.array([(k//11)%3==fold for k in keys]);c=np.linalg.lstsq(x[~test],y[~test],rcond=None)[0];held[test]=x[test]@c
  models[name]={'coefficients':coeff.tolist(),'training_r_squared':float(1-np.sum((y-pred)**2)/np.sum((y-y.mean())**2)),'held_block_mae':float(np.mean(abs(y-held))),'prediction_range':[float(pred.min()),float(pred.max())]}
 raw=np.array(samples,dtype=float);matched=[]
 for owner in [0,1]:
  for u in range(pattern.shape[1]):
   for v in range(pattern.shape[0]):
    selected=raw[(raw[:,3]==owner)&(raw[:,12]==u)&(raw[:,13]==v)];up=selected[selected[:,4]==0];lo=selected[selected[:,4]==1]
    if len(up)>=3 and len(lo)>=3:matched.append({'wing':owner,'fixed_uv_pixel':[u,v],'upper_samples':len(up),'lower_samples':len(lo),'upper_phases':len(set(up[:,0])),'lower_phases':len(set(lo[:,0])),'upper_median_rgb':np.median(up[:,6:9],axis=0).tolist(),'lower_median_rgb':np.median(lo[:,6:9],axis=0).tolist()})
 selected_phases=[2,8,18,21,27,29,30,47,63,75,87,93];sheet=Image.new('RGB',(1000,180*len(selected_phases)),'#252525');draw=ImageDraw.Draw(sheet)
 for index,phase in enumerate(selected_phases):
  source=Image.open(fit['poses'][phase]['source']['source']).convert('RGBA');top=index*180;draw.text((8,top+5),f'Phase{phase}: original / conditional physical side (blue upper, orange lower)',fill='white')
  for column,image in enumerate([source,owner_images[phase]]):image=image.resize((image.width*10,image.height*10),Image.Resampling.NEAREST);sheet.paste(image,(25+column*200,top+30),image)
  r=rows[phase];draw.text((460,top+35),f"Wing cosines: {np.round(r['simple_signed_wing_cosines'],3)}",fill='white')
  for j,label in enumerate(['white_upper','white_lower','interior_white_upper','interior_white_lower']):
   q=r['observed_color_by_conditional_side'][label];value=q['mean_linear_luminance'];draw.text((460,top+58+j*21),f"{label}: n{q['count']}, linearY {round(value,3) if value is not None else '-'}",fill='white')
 sheet.save(OUT/'source-and-conditional-side.png')
 totals={name:sum(r['observed_color_by_conditional_side'][name]['count'] for r in rows) for name in rows[0]['observed_color_by_conditional_side']};report={'status':'READ_ONLY_CONDITIONAL_MATERIAL_EVIDENCE','model_sha256':sha(BASE/'rig-joint-trial-v1/model.blend'),'fit_sha256':sha(fitpath),'fixed_pattern_sha256':authority['fixed_pattern_sha256'],'method':['Exact front/back/edge triangle fan and body topology reconstructed from conserved recipe, ray-tested at native source positive pixel centers.','Classify first physical hit, not merely a negative wing normal; exclude grazing/edge/body and rendered-alpha disagreements from material comparison.','Fixed phase2 anatomical UV sampling labels expected white pattern. Colors are original frame RGB, never repainted.','Upper is the constructed side facing the camera in phase2, not proven biological dorsal/ventral identity.','Color models use99 source phases with11phase temporal blocks held out; they are descriptive hypotheses, not production shading.'],'totals':totals,'phases':rows,'phase_white_ratio':phase_groups,'descriptive_models':models,'cross_side_matched_uv_bins':matched,'sample_schema':['phase','source_x','source_y','wing0left1right','side0upper1lower','distance_to_alpha_edge','r','g','b','fixed_r','fixed_g','fixed_b','fixed_u','fixed_v','nx','ny','nz','fixed_pattern_white'],'limitations':['The current poses are fitted hypotheses, including uncertain18/30; their side labels cannot independently prove pigment identity.','Near-boundary opaque RGB may include original filtering/background-color contamination; binary alpha does not recover fractional opacity.','No per-frame texture or model/material edit; no exact preservation claim for physical rendering.']};(OUT/'report.json').write_text(json.dumps(report,indent=2)+'\n');(OUT/'samples.json').write_text(json.dumps(samples,separators=(',',':'))+'\n');print(json.dumps({'totals':totals,'models':models,'matched_bins':len(matched)},indent=2))
if __name__=='__main__':main()
