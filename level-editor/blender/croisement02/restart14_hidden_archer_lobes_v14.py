"""CPU lobe arrangement: finite ellipsoid volumes, native rays, local supports."""
import json,math,hashlib,shutil
from pathlib import Path
import numpy as np
from PIL import Image
from scipy.spatial import cKDTree
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
BASE=Path(__file__).resolve().parents[2]/'work/croisement02-refinement/restart14-hidden-archer';DEST=BASE/'lobes-v14-cpu';SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35));RAY=np.array([0,-COS,SIN]);CAP=2*1024**2

def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def budget():
 used=sum(p.stat().st_size for p in DEST.rglob('*') if p.is_file()) if DEST.exists() else 0
 assert used<CAP;assert shutil.disk_usage(BASE).free>=8*1024**3+CAP-used

def nearest_origin(tri):
 a=tri[:,0];e=tri[:,1]-a;f=tri[:,2]-a;ee=np.einsum('ij,ij->i',e,e);ef=np.einsum('ij,ij->i',e,f);ff=np.einsum('ij,ij->i',f,f);ae=np.einsum('ij,ij->i',a,e);af=np.einsum('ij,ij->i',a,f);den=ee*ff-ef*ef;u=np.divide(-ae*ff+af*ef,den,out=np.zeros(len(a)),where=np.abs(den)>1e-20);v=np.divide(-af*ee+ae*ef,den,out=np.zeros(len(a)),where=np.abs(den)>1e-20);p=a+u[:,None]*e+v[:,None]*f;valid=(np.abs(den)>1e-20)&(u>=0)&(v>=0)&(u+v<=1);dist=np.where(valid,np.einsum('ij,ij->i',p,p),np.inf)
 for i,j in [(0,1),(1,2),(2,0)]:
  x=tri[:,i];edge=tri[:,j]-x;length=np.einsum('ij,ij->i',edge,edge);t=np.clip(np.divide(-np.einsum('ij,ij->i',x,edge),length,out=np.zeros(len(x)),where=length>1e-20),0,1);q=x+t[:,None]*edge;d=np.einsum('ij,ij->i',q,q);better=d<dist;p[better]=q[better];dist=np.minimum(dist,d)
 return dist,p

def main():
 budget();assert not DEST.exists();data=np.load(BASE/'surface-v8/surfaces.npz');rock=data['vertices0'][data['triangles0']];bank=data['vertices1'][data['triangles1']];centroids=rock.mean(1);radii=np.linalg.norm(rock-centroids[:,None],axis=2).max(1);index=cKDTree(centroids);vertex=cKDTree(data['vertices0']);max_radius=float(radii.max())
 def clearance(point):
  upper=vertex.query(point)[0];ids=index.query_ball_point(point,upper+max_radius+1e-6);tri=rock[ids];d,q=nearest_origin(tri-point);i=int(np.argmin(d));normal=np.cross(tri[i,1]-tri[i,0],tri[i,2]-tri[i,0]);signed=math.sqrt(d[i])*(1 if -q[i]@normal>=-1e-8 else -1);bd,bq=nearest_origin(bank-point);j=int(np.argmin(bd));bn=np.cross(bank[j,1]-bank[j,0],bank[j,2]-bank[j,0]);bs=math.sqrt(bd[j])*(1 if -bq[j]@bn>=-1e-8 else -1);normal/=np.linalg.norm(normal);return min(signed,bs),normal
 def ellipsoid_clear(center,axes):
  ids=index.query_ball_point(center,np.linalg.norm(axes,axis=0).max()+max_radius+.02);inverse=np.linalg.inv(axes);rt=(rock[ids]-center)@inverse.T;bt=(bank-center)@inverse.T;minimum=min(float(nearest_origin(rt)[0].min()) if len(rt) else 1e9,float(nearest_origin(bt)[0].min()));return minimum>1.015**2,math.sqrt(minimum)
 def ray_hits(pixels,center,axes):
  starts=np.column_stack([pixels[:,0]+.5,-(pixels[:,1]+.5)/SIN,np.zeros(len(pixels))]);inverse=np.linalg.inv(axes);q=(starts-center)@inverse.T;d=RAY@inverse.T;a=d@d;b=2*(q@d);c=np.einsum('ij,ij->i',q,q)-1;disc=b*b-4*a*c;valid=disc>=0;depth=np.full(len(pixels),np.nan);depth[valid]=(-b[valid]+np.sqrt(disc[valid]))/(2*a);return valid,depth
 attachments=json.loads((BASE/'skeleton-v9-cpu/root-attachment-final-centers.json').read_text());DEST.mkdir();fig,axs=plt.subplots(2,2,figsize=(11,10),layout='constrained');reports=[]
 for row,state in enumerate(['initial','applied']):
  pp=BASE/f'skeleton-v9-cpu/{state}-plan.json';plan=json.loads(pp.read_text());pixels=np.array(plan['pixels']);front=np.array(plan['front']);chains=[front[c]-RAY*.6 for c in plan['segments']];branch_points=np.vstack(chains);btree=cKDTree(branch_points);covered=np.zeros(len(front),bool);lobes=[];rejected=[];rng=np.random.default_rng(801+row)
  # Greedy uncovered-source seeds create irregular overlapping lobes, not a
  # uniform-offset surface. Every accepted solid ellipsoid excludes receivers.
  attempts=0
  while (~covered).any() and len(lobes)<72 and attempts<120:
   remaining=np.where(~covered)[0];seed=int(remaining[np.argmax(np.minimum(np.linalg.norm(front[remaining]-front[remaining].mean(0),axis=1),25))]) if not lobes else int(remaining[np.argmin(btree.query(front[remaining])[0])]);attempts+=1;anchor=front[seed];d0,normal=clearance(anchor);free=d0>8
   if normal@RAY<0:normal=-normal
   u=np.cross(normal,[0,0,1]);u/=max(np.linalg.norm(u),1e-9);v=np.cross(normal,u);angle=float(rng.uniform(-.8,.8));u,v=u*math.cos(angle)+v*math.sin(angle),-u*math.sin(angle)+v*math.cos(angle);accepted=None
   for scale in [1.,.8,.6]:
    radius=np.array([6.5,5.5,5.0 if free else 2.3])*scale
    for offset in [2.,5.,8.,11.]:
     center=anchor+RAY*offset;axes=np.column_stack([u*radius[0],v*radius[1],normal*radius[2]]);clear,minimum=ellipsoid_clear(center,axes)
     if not clear:continue
     dist,bi=btree.query(center);base=branch_points[bi];toward=base-center;unit=np.linalg.solve(axes,toward);tip=center+toward/max(np.linalg.norm(unit),1)*.85;reach=float(np.linalg.norm(tip-base))
     if reach>16:continue
     count=max(2,int(math.ceil(reach/.6))+1);margin=.12+reach/(count-1)/2;twig_lower=min(clearance(base+t*(tip-base))[0]-margin for t in np.linspace(0,1,count))
     if twig_lower<0:continue
     hit,depth=ray_hits(pixels,center,axes)
     if not hit[seed]:continue
     accepted=dict(seed_pixel=pixels[seed].tolist(),center=center.tolist(),axes=axes.tolist(),radii=radius.tolist(),surface_clearance_normalized=minimum,twig_base=base.tolist(),twig_tip=tip.tolist(),twig_length=reach,twig_clearance_lower_bound=twig_lower,native_rays=int(hit.sum()),inference='Own-source leaf packets around cleared branches; this solid lobe is an envelope, not a rendered solid blob.');covered|=hit;lobes.append(accepted);break
    if accepted:break
   if accepted is None:
    rejected.append(int(seed));covered[seed]=True # Excluded from seed retry only; recomputed real coverage below.
  real=np.zeros(len(front),bool)
  for l in lobes:real|=ray_hits(pixels,np.array(l['center']),np.array(l['axes']))[0]
  rgba=np.array(Image.open(plan['source']));ox,oy=plan['source_top_left'];axs[row,0].imshow(rgba,extent=[ox,ox+rgba.shape[1],oy+rgba.shape[0],oy]);axs[row,0].scatter(pixels[~real,0]+.5,pixels[~real,1]+.5,c='red',s=3)
  for l in lobes:
   c=np.array(l['center']);axs[row,0].plot(c[0],-c[1]*SIN-c[2]*COS,'o',mfc='none',mec='cyan',ms=6)
   axes=np.array(l['axes']);angles=np.linspace(0,math.tau,60);ring=c+np.outer(np.cos(angles),axes[:,0])+np.outer(np.sin(angles),axes[:,2]);axs[row,1].plot(ring[:,1],ring[:,2],lw=.6);twig=np.array([l['twig_base'],l['twig_tip']]);axs[row,1].plot(twig[:,1],twig[:,2],c='#774421',lw=.4)
  axs[row,0].set_title(state+': lobe seeds; red unresolved native centers');axs[row,1].set_title('Independent cleared lobe sections/local twigs');axs[row,1].set_aspect('equal');axs[row,1].set_xlabel('World Y');axs[row,1].set_ylabel('World Z')
  report=dict(state=state,status='ARRANGEMENT ONLY' if real.all() else 'HOLD; native coverage incomplete',plan_sha256=sha(pp),source_sha256=plan['source_sha256'],lobes=lobes,native_centers=len(front),covered_native_centers=int(real.sum()),uncovered_native_pixels=pixels[~real].tolist(),failed_seed_count=len(rejected),off_map_strategy='Continue lobes whose native footprint reaches y0 with decreasing side/back leaf density up to12 source pixels beyond map; gray own-source-alpha packets, no planar cap. Exact off-map packet geometry still pending.',leaf_packet_recipe='Use individually tilted1–3pixel front packets placed at accepted lobe ray intersections; source UV unchanged. Seed side/rear leaf fans around ellipsoid surfaces with original endpoint opacity guard and explicit inferred material ownership. No continuous source-grid surface.',limits=['Analytic ellipsoid/triangle exclusion certifies envelope volumes; actual leaf polygons and native texture first hits require the model guard.','Lobe source coverage and organic off-map topology must be complete before model readiness.','No new images/textures synthesized; only native source and existing receiver geometry used.'])
  budget();(DEST/f'{state}-arrangement.json').write_text(json.dumps(report,indent=2)+'\n');reports.append({k:v for k,v in report.items() if k not in ['lobes','uncovered_native_pixels','limits','leaf_packet_recipe','off_map_strategy']})
 budget();fig.savefig(DEST/'lobes-and-source.png',dpi=100);plt.close(fig);budget();(DEST/'summary.json').write_text(json.dumps(dict(states=reports,surfaces_sha256=sha(BASE/'surface-v8/surfaces.npz'),image_sha256=sha(DEST/'lobes-and-source.png')),indent=2)+'\n');print(json.dumps(reports,indent=2))
if __name__=='__main__':main()
