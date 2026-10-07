"""Selected-surface tube clearance before any climbing candidate model write."""
import sys,json,math,shutil
from pathlib import Path
import bpy,numpy as np
from mathutils.bvhtree import BVHTree
from mathutils import Vector
from scipy.spatial import cKDTree
from scipy.ndimage import gaussian_filter1d
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import RAY
BASE=OUT/'restart14-hidden-archer';DEST=BASE/'volume-v9-guard'
def main():
 assert shutil.disk_usage(BASE).free>10*1024**3+16*1024**2;assert not DEST.exists();bpy.ops.wm.read_factory_settings(use_empty=True);bpy.context.scene.render.threads_mode='FIXED';bpy.context.scene.render.threads=2
 data=np.load(BASE/'surface-v8/surfaces.npz');v=data['vertices0'];tri=data['triangles0'];rock=BVHTree.FromPolygons(v.tolist(),tri.tolist(),all_triangles=True);bank=BVHTree.FromPolygons(data['vertices1'].tolist(),data['triangles1'].tolist(),all_triangles=True);q=v[tri];normals=np.zeros_like(v);fn=np.cross(q[:,1]-q[:,0],q[:,2]-q[:,0])
 for k in range(3):np.add.at(normals,tri[:,k],fn)
 normals/=np.maximum(np.linalg.norm(normals,axis=1)[:,None],1e-12);vertex_tree=cKDTree(v);surface_paths=json.loads((BASE/'geodesic-v8-cpu/report.json').read_text())['paths'];attachments=json.loads((BASE/'skeleton-v9-cpu/root-attachment-final-centers.json').read_text());guides=[]
 for k,g in enumerate(surface_paths):
  surface=np.array(g['points']);d=gaussian_filter1d(normals[vertex_tree.query(surface)[1]],1.5,axis=0);d/=np.maximum(np.linalg.norm(d,axis=1)[:,None],1e-12);points=surface+d*4;guides.append(np.vstack([attachments['states'][0]['rock_path_roots'][k],points]))
 records=[]
 for state in ['initial','applied']:
  plan=json.loads((BASE/f'skeleton-v9-cpu/{state}-plan.json').read_text());attach=next(a for a in attachments['states'] if a['state']==state);front=np.array(plan['front']);centers=front-np.array(RAY)*.6;chains=guides+[centers[c] for c in plan['segments']]+[np.array(attach['climber_join']),np.array(attach['right_branch'])];fail=[];minimum=1e9;tested=0
  for ci,chain in enumerate(chains):
   radius=.65 if ci<3 or ci==len(chains)-1 else .18
   for si,(a,b) in enumerate(zip(chain[:-1],chain[1:])):
    length=np.linalg.norm(b-a);count=max(2,int(math.ceil(length/.25))+1);spacing=length/(count-1);lower=1e9;inside=0;bank_inside=0
    for t in np.linspace(0,1,count):
     p=a+(b-a)*t;near,n,idx,distance=rock.find_nearest(Vector(p));signed=distance if (Vector(p)-near).dot(n)>=-1e-6 else -distance;lower=min(lower,signed-radius-spacing/2);inside+=signed<0;tested+=1
     bn,normal,_,bd=bank.find_nearest(Vector(p));bs=bd if (Vector(p)-bn).dot(normal)>=-1e-6 else -bd
     # Only the first rooted span may deliberately enter the bank by0.15.
     rooted=(ci<3 or ci==len(chains)-1) and si==0
     if bs < (-.16 if rooted else radius+spacing/2):bank_inside+=1
    minimum=min(minimum,lower)
    if lower<0 or bank_inside:fail.append(dict(chain=ci,segment=si,rock_clearance_lower_bound=lower,rock_interior_samples=int(inside),bank_nonclear_samples=int(bank_inside),radius=radius))
  records.append(dict(state=state,center_samples=tested,failed_segments=len(fail),minimum_rock_clearance_lower_bound=minimum,failures=fail,limitations=['Distance minus radius minus half-step is a conservative complete-segment exterior clearance bound when sign is consistent.','Rooted bank penetration is limited to first segment,0.15 center depth; actual cap/soil union still needs final mesh audit.','This tests planned stem volumes only; leaf volume/native material visibility remain separate gates.']))
 assert shutil.disk_usage(BASE).free>10*1024**3+1048576;DEST.mkdir();write_json(DEST/'report.json',dict(status='HOLD' if any(r['failed_segments'] for r in records) else 'STEM CLEARANCE PASS',records=records,surface_sha256=sha(BASE/'surface-v8/surfaces.npz'),attachments_sha256=sha(BASE/'skeleton-v9-cpu/root-attachment-final-centers.json'),model_created=False));print(json.dumps([{k:v for k,v in r.items() if k not in ['failures','limitations']} for r in records],indent=2))
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
