"""CPU definition of bounded soil-embedded collars, without changing branches."""
import json,hashlib,math,shutil
from pathlib import Path
import numpy as np
from scipy.spatial import cKDTree
from scipy.ndimage import gaussian_filter1d
BASE=Path(__file__).resolve().parents[2]/'work/croisement02-refinement/restart14-hidden-archer';DEST=BASE/'collar-v11-cpu'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 assert shutil.disk_usage(BASE).free>8*1024**3+2*1024**2;assert not DEST.exists()
 archive=BASE/'surface-v8/surfaces.npz';data=np.load(archive);v=data['vertices0'];tri=data['triangles0'];q=v[tri];n=np.zeros_like(v);fn=np.cross(q[:,1]-q[:,0],q[:,2]-q[:,0])
 for k in range(3):np.add.at(n,tri[:,k],fn)
 n/=np.maximum(np.linalg.norm(n,axis=1)[:,None],1e-12);tree=cKDTree(v);bank=data['vertices1'][data['triangles1']];attachment=BASE/'skeleton-v9-cpu/root-attachment-final-centers.json';a=json.loads(attachment.read_text());guides=json.loads((BASE/'geodesic-v8-cpu/report.json').read_text())['paths'];chains=[]
 def height(x,y):
  p=np.array([x,y]);va=bank[:,0,:2];e1=bank[:,1,:2]-va;e2=bank[:,2,:2]-va;s=p-va;det=e1[:,0]*e2[:,1]-e1[:,1]*e2[:,0];good=np.abs(det)>1e-12;u=np.divide(s[:,0]*e2[:,1]-s[:,1]*e2[:,0],det,out=np.zeros(len(det)),where=good);w=np.divide(e1[:,0]*s[:,1]-e1[:,1]*s[:,0],det,out=np.zeros(len(det)),where=good);inside=good&(u>=-1e-8)&(w>=-1e-8)&(u+w<=1+1e-8);assert inside.any();z=bank[:,0,2]+u*(bank[:,1,2]-bank[:,0,2])+w*(bank[:,2,2]-bank[:,0,2]);return float(z[inside].max())
 for k,g in enumerate(guides):
  surface=np.array(g['points']);direction=gaussian_filter1d(n[tree.query(surface)[1]],1.5,axis=0);direction/=np.maximum(np.linalg.norm(direction,axis=1)[:,None],1e-12);chains.append(np.vstack([a['states'][0]['rock_path_roots'][k],surface+direction*4]))
 chains.append(np.array(a['states'][0]['right_branch']));records=[]
 for k,chain in enumerate(chains):
  root=chain[0];z=height(*root[:2]);collar=dict(root_index=k,root_world=root.tolist(),bank_height=z,center_xy=root[:2].tolist(),footprint_radius=2.,bottom_z=z-1.1,top_z=z+1.5,stem_radius=.65,maximum_center_sampling_step=.25);embed=[];bad=[]
  for si,(p0,p1) in enumerate(zip(chain[:-1],chain[1:])):
   count=max(2,int(math.ceil(np.linalg.norm(p1-p0)/.25))+1);margin=.65+np.linalg.norm(p1-p0)/(count-1)/2
   for t in np.linspace(0,1,count):
    p=p0+t*(p1-p0);bank_z=height(*p[:2]);needs_collar=p[2]-bank_z<margin
    if needs_collar:
     within=(np.linalg.norm(p[:2]-root[:2])+margin<=2 and p[2]-margin>=collar['bottom_z'] and p[2]+margin<=collar['top_z']);row=dict(segment=si,world=p.tolist(),swept_radius_bound=float(margin),soil_depth_bound=float(bank_z-p[2]+margin));embed.append(row)
     if not within:bad.append(row)
  collar.update(embedded_sample_count=len(embed),maximum_soil_depth_bound=max([r['soil_depth_bound'] for r in embed],default=0),failed_scope_samples=bad,embedded_samples=embed);records.append(collar)
 report=dict(status='CPU COLLAR PLAN PASS' if not any(r['failed_scope_samples'] for r in records) else 'HOLD',surface_sha256=sha(archive),attachment_sha256=sha(attachment),parent_guard_sha256=sha(BASE/'volume-v10-guard/report.json'),collars=records,branch_coordinates_changed=False,native_front_coordinates_changed=False,scope=['Only tube portions fully enclosed by these measured 2-unit-radius collars may intersect the soil.','No segment-index exception. Outside each collar, the complete visible stalk must pass signed surface clearance minus radius and half sampling step.','Up to1.1 units of underground tube base is explicitly inferred and embedded; this is not a detached/floating contact approximation.','Exact BVH finite-radius inspection still required before candidate construction.'])
 payload=(json.dumps(report,indent=2)+'\n').encode();assert len(payload)<2*1024**2;assert shutil.disk_usage(BASE).free>8*1024**3+2*1024**2;DEST.mkdir();(DEST/'collars.json').write_bytes(payload);print(report['status']);print([(r['root_index'],r['embedded_sample_count'],r['maximum_soil_depth_bound'],len(r['failed_scope_samples'])) for r in records])
if __name__=='__main__':main()
