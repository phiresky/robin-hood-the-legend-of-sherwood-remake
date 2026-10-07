"""CPU-only exact triangle tests for sparse climbing root attachment proposals."""
import hashlib,json,math,shutil
from pathlib import Path
import numpy as np
from scipy.spatial import cKDTree
from scipy.ndimage import gaussian_filter1d
BASE=Path(__file__).resolve().parents[2]/'work/croisement02-refinement/restart14-hidden-archer';OUT=BASE/'skeleton-v9-cpu/root-attachment-final-centers.json';RAY=np.array([0,-math.cos(math.radians(35)),math.sin(math.radians(35))])
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def intersections(a,b,triangles):
 # Two-sided segment/triangle test, including endpoints except explicit epsilon.
 lo=np.minimum(a,b)-1e-6;hi=np.maximum(a,b)+1e-6
 idx=np.where((BOUNDS[1]>=lo).all(1)&(BOUNDS[0]<=hi).all(1))[0]
 t=triangles[idx];direction=b-a;e1=t[:,1]-t[:,0];e2=t[:,2]-t[:,0];h=np.cross(direction,e2);det=np.einsum('ij,ij->i',e1,h);ok=np.abs(det)>1e-10;inv=np.divide(1,det,out=np.zeros_like(det),where=ok);s=a-t[:,0];u=inv*np.einsum('ij,ij->i',s,h);q=np.cross(s,e1);v=inv*(q@direction);distance=inv*np.einsum('ij,ij->i',e2,q);hit=ok&(u>=-1e-7)&(v>=-1e-7)&(u+v<=1+1e-7)&(distance>1e-5)&(distance<1-1e-5)
 return idx[hit].tolist()
def main():
 assert not OUT.exists();assert shutil.disk_usage(BASE).free>8*1024**3+2*1024**2
 global BOUNDS
 archive=BASE/'surface-v8/surfaces.npz';data=np.load(archive);rock=data['vertices0'][data['triangles0']];bank=data['vertices1'][data['triangles1']];BOUNDS=(rock.min(1),rock.max(1));gpath=BASE/'geodesic-v8-cpu/report.json';guides=json.loads(gpath.read_text())['paths'];all_guides=[];ground_roots=[]
 vertices=data['vertices0'];tri=data['triangles0'];normals=np.zeros_like(vertices);fn=np.cross(rock[:,1]-rock[:,0],rock[:,2]-rock[:,0])
 for k in range(3):np.add.at(normals,tri[:,k],fn)
 normals/=np.maximum(np.linalg.norm(normals,axis=1)[:,None],1e-12);vertex_tree=cKDTree(vertices)
 def bank_height(x,y):
  p=np.array([x,y]);a=bank[:,0,:2];e1=bank[:,1,:2]-a;e2=bank[:,2,:2]-a;s=p-a;det=e1[:,0]*e2[:,1]-e1[:,1]*e2[:,0];ok=np.abs(det)>1e-12
  u=np.divide(s[:,0]*e2[:,1]-s[:,1]*e2[:,0],det,out=np.zeros(len(det)),where=ok);v=np.divide(e1[:,0]*s[:,1]-e1[:,1]*s[:,0],det,out=np.zeros(len(det)),where=ok);inside=ok&(u>=0)&(v>=0)&(u+v<=1);heights=bank[:,0,2]+u*(bank[:,1,2]-bank[:,0,2])+v*(bank[:,2,2]-bank[:,0,2]);assert inside.any();return float(heights[inside].max())
 for guide in guides:
  surface=np.array(guide['points']);nearest=vertex_tree.query(surface)[1];direction=gaussian_filter1d(normals[nearest],1.5,axis=0);direction/=np.maximum(np.linalg.norm(direction,axis=1)[:,None],1e-12);points=surface+direction*4.;root=points[0].copy();root[2]=bank_height(*root[:2])-.15;points=np.vstack([root,points]);all_guides.append(points);ground_roots.append(root.tolist())
 reports=[]
 for state in ['initial','applied']:
  planpath=BASE/f'skeleton-v9-cpu/{state}-plan.json';p=json.loads(planpath.read_text());front=np.array(p['front']);centers=front-RAY*.6;branches=[centers[chain] for chain in p['segments']];root=centers[p['root_index']]
  pooled=np.vstack(all_guides);target=pooled[np.argmin(np.linalg.norm(pooled-root,axis=1))];branches.append(np.array([target,root]))
  side_index=int(np.argmin(np.linalg.norm(front-[175,-240,120],axis=1)));side=centers[side_index];side_root=np.array([175.,-190.,bank_height(175.,-190.)-.15]);side_curve=np.array([side_root,[175,-197,65],[175,-214,95],side]);branches.append(side_curve)
  tested=all_guides+branches;fail=[];count=0
  for chain_index,chain in enumerate(tested):
   for j,(a,b) in enumerate(zip(chain[:-1],chain[1:])):
    hits=intersections(a,b,rock);count+=1
    if hits:fail.append(dict(chain=chain_index,segment=j,rock_triangles=hits[:8],a=a.tolist(),b=b.tolist()))
  reports.append(dict(state=state,plan_sha256=sha(planpath),segments_tested=count,rock_crossing_segments=len(fail),crossings=fail,rock_path_roots=ground_roots,right_shrub_root=side_root.tolist(),right_branch=side_curve.tolist(),climber_join=[target.tolist(),root.tolist()],root_bank_embed=.15,limits=['Exact centerline triangle crossings only; zero crossings cannot certify tube radius clearance or starting outside closed rock.','Bank root height is independently interpolated from selected evaluated triangles, not a nominal flat constant.','Native visibility of new stems and tiny detached leaf components remains to be checked in a real candidate.']))
 report=dict(status='HOLD' if any(r['rock_crossing_segments'] for r in reports) else 'CENTERLINES CLEAR; VOLUME CLEARANCE STILL PENDING',guide_offset='4 units along smoothed area-weighted outward normals; exact triangle crossing checked',surface_sha256=sha(archive),geodesic_sha256=sha(gpath),states=reports,model_created=False)
 payload=json.dumps(report,indent=2)+'\n';assert len(payload)<256*1024;assert sum(p.stat().st_size for p in OUT.parent.iterdir() if p.is_file())+len(payload)<2*1024**2;OUT.write_text(payload);print(json.dumps([{k:v for k,v in r.items() if k not in ['crossings','limits','right_branch','climber_join','rock_path_roots']} for r in reports],indent=2))
if __name__=='__main__':main()
