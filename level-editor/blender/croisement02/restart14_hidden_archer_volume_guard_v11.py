"""Versioned 2 MiB clearance inspection under the explicit small-job policy."""
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
BASE=OUT/'restart14-hidden-archer';DEST=BASE/'volume-v11-guard'
POLICY=OUT/'restart17-small-job-disk-policy.json'
POLICY_SHA='4bb7da826f59dc05ca8bd7654babed5bde622a8b52d63a81e8098f302ced45e2'
PARENT_SHA='0204abd871ebf3fbbba20105c507776ff91030cd57fd3df0cea8c715dd4a29e5'
CAP=2*1024**2
def budget():
 assert sha(POLICY)==POLICY_SHA
 policy=json.loads(POLICY.read_text());assert policy['status']=='ROOT_AUTHORIZED_BOUNDED_INSPECTION_EXCEPTION'
 used=sum(p.stat().st_size for p in DEST.rglob('*') if p.is_file()) if DEST.exists() else 0
 assert used<=CAP
 assert shutil.disk_usage(BASE).free>=policy['minimum_free_bytes']+(CAP-used)

def main():
 budget();assert sha(HERE/'restart14_hidden_archer_volume_guard_v10.py')==PARENT_SHA;assert not DEST.exists();bpy.ops.wm.read_factory_settings(use_empty=True);bpy.context.scene.render.threads_mode='FIXED';bpy.context.scene.render.threads=2
 data=np.load(BASE/'surface-v8/surfaces.npz');v=data['vertices0'];tri=data['triangles0'];rock=BVHTree.FromPolygons(v.tolist(),tri.tolist(),all_triangles=True);bank=BVHTree.FromPolygons(data['vertices1'].tolist(),data['triangles1'].tolist(),all_triangles=True);q=v[tri];normals=np.zeros_like(v);fn=np.cross(q[:,1]-q[:,0],q[:,2]-q[:,0])
 for k in range(3):np.add.at(normals,tri[:,k],fn)
 normals/=np.maximum(np.linalg.norm(normals,axis=1)[:,None],1e-12);vertex_tree=cKDTree(v);surface_paths=json.loads((BASE/'geodesic-v8-cpu/report.json').read_text())['paths'];attachments=json.loads((BASE/'skeleton-v9-cpu/root-attachment-final-centers.json').read_text());guides=[]
 for k,g in enumerate(surface_paths):
  surface=np.array(g['points']);d=gaussian_filter1d(normals[vertex_tree.query(surface)[1]],1.5,axis=0);d/=np.maximum(np.linalg.norm(d,axis=1)[:,None],1e-12);points=surface+d*4;guides.append(np.vstack([attachments['states'][0]['rock_path_roots'][k],points]))
 collar_path=BASE/'collar-v11-cpu/collars.json';assert sha(collar_path)=='49fadb746412b8d55765dd583e54ef0c6ec7e967bc8a57e3a1e90ace0e5565b8';collar_plan=json.loads(collar_path.read_text());assert collar_plan['status']=='CPU COLLAR PLAN PASS';collars=collar_plan['collars']
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
     # Soil intersection is allowed only inside a measured finite collar.
     collar=collars[ci] if ci<3 else collars[3] if ci==len(chains)-1 else None
     margin=radius+spacing/2
     scoped=bool(collar and np.linalg.norm(p[:2]-np.array(collar['center_xy']))+margin<=collar['footprint_radius'] and p[2]-margin>=collar['bottom_z'] and p[2]+margin<=collar['top_z'])
     if bs < margin and not scoped:bank_inside+=1
    minimum=min(minimum,lower)
    if lower<0 or bank_inside:fail.append(dict(chain=ci,segment=si,rock_clearance_lower_bound=lower,rock_interior_samples=int(inside),bank_nonclear_samples=int(bank_inside),radius=radius))
  records.append(dict(state=state,center_samples=tested,failed_segments=len(fail),minimum_rock_clearance_lower_bound=minimum,failures=fail,limitations=['Distance minus radius minus half-step is a conservative complete-segment exterior clearance bound when sign is consistent.','Only tube portions completely enclosed by measured collar bounds may intersect bank; exposed stalk has no segment-index exemption.','This tests planned stem volumes only; leaf volume/native material visibility remain separate gates.']))
 budget();report=dict(status='HOLD' if any(r['failed_segments'] for r in records) else 'STEM CLEARANCE PASS',records=records,surface_sha256=sha(BASE/'surface-v8/surfaces.npz'),attachments_sha256=sha(BASE/'skeleton-v9-cpu/root-attachment-final-centers.json'),model_created=False,root_collar_sha256=sha(collar_path),branch_coordinates_changed=False,native_front_coordinates_changed=False,policy=str(POLICY),policy_sha256=POLICY_SHA,parent_recipe_sha256=PARENT_SHA,output_cap_bytes=CAP,minimum_free_bytes=8*1024**3);payload=(json.dumps(report,indent=2)+'\n').encode();assert len(payload)<=CAP;budget();DEST.mkdir();(DEST/'report.json').write_bytes(payload);print(json.dumps([{k:v for k,v in r.items() if k not in ['failures','limitations']} for r in records],indent=2))
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
