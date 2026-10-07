"""Bounded sparse climbing endpoints with actual leaf-surface clearance gates."""
import sys,json,math,shutil,os
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from scipy.spatial import cKDTree
from scipy.ndimage import gaussian_filter1d,map_coordinates
from mathutils import Vector
from mathutils.bvhtree import BVHTree
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from tree_geometry import RAY,SIN,COS,material,one_sided,replace_mesh
from refinement_review import _tree
from evidence_io import sha,write_json
from render_slots import acquire,release
BASE=OUT/'restart14-hidden-archer';DEST=BASE/'climbing-v13';CAP=32*1024**2
POLICY=OUT/'restart17-small-job-disk-policy.json';POLICY_SHA='4bb7da826f59dc05ca8bd7654babed5bde622a8b52d63a81e8098f302ced45e2'
def budget(extra=0):
 assert sha(POLICY)==POLICY_SHA
 used=sum(p.stat().st_size for p in DEST.rglob('*') if p.is_file()) if DEST.exists() else 0
 assert used+extra<=CAP;assert shutil.disk_usage(BASE).free>=8*1024**3+CAP-used

def main():
 budget();assert not DEST.exists();guard=BASE/'volume-v11-guard/report.json';g=json.loads(guard.read_text());assert g['status']=='STEM CLEARANCE PASS';assert sha(guard)=='3c72b4aedaa1b1e82cf35ee708f2240034d764dd59befe3b8280e1c46665fa2d'
 data=np.load(BASE/'surface-v8/surfaces.npz');rv=data['vertices0'];rt=data['triangles0'];rock=BVHTree.FromPolygons(rv.tolist(),rt.tolist(),all_triangles=True);bank=BVHTree.FromPolygons(data['vertices1'].tolist(),data['triangles1'].tolist(),all_triangles=True);q=rv[rt];normals=np.zeros_like(rv);fn=np.cross(q[:,1]-q[:,0],q[:,2]-q[:,0])
 for k in range(3):np.add.at(normals,rt[:,k],fn)
 normals/=np.maximum(np.linalg.norm(normals,axis=1)[:,None],1e-12);tree=cKDTree(rv);paths=json.loads((BASE/'geodesic-v8-cpu/report.json').read_text())['paths'];attachments=json.loads((BASE/'skeleton-v9-cpu/root-attachment-final-centers.json').read_text());guides=[]
 for k,p in enumerate(paths):
  points=np.array(p['points']);n=gaussian_filter1d(normals[tree.query(points)[1]],1.5,axis=0);n/=np.maximum(np.linalg.norm(n,axis=1)[:,None],1e-12);guides.append(np.vstack([attachments['states'][0]['rock_path_roots'][k],points+n*4]))
 envelope=np.load(BASE/'skeleton-v9-cpu/envelope.npz');field=envelope['height'];origin=envelope['origin'];ray=np.array(RAY);DEST.mkdir();all_pass=True
 def signed_distance(p,bvh):
  near,n,_,d=bvh.find_nearest(Vector(p));return d if (Vector(p)-near).dot(n)>=-1e-6 else -d
 def clear_polygon(points,margin=.04):
  # Barycentric samples cover each triangle; distance is 1-Lipschitz, so
  # subtracting the sample-cell diameter is conservative over the full face.
  worst=1e9
  for i in range(1,len(points)-1):
   t=np.array([points[0],points[i],points[i+1]]);edge=max(np.linalg.norm(t[1]-t[0]),np.linalg.norm(t[2]-t[0]),np.linalg.norm(t[2]-t[1]));n=max(2,int(math.ceil(edge/.35)));cover=2*edge/n
   for a in range(n+1):
    for b in range(n+1-a):
     p=t[0]+(t[1]-t[0])*(a/n)+(t[2]-t[0])*(b/n);d=min(signed_distance(p,rock),signed_distance(p,bank));worst=min(worst,d-cover)
     if d<margin:return False,worst
  return worst>=margin,worst
 for state in ['initial','applied']:
  folder=DEST/f'profile-05-{state}';budget();folder.mkdir();planpath=BASE/f'skeleton-v9-cpu/{state}-plan.json';plan=json.loads(planpath.read_text());source=Path(plan['source']);assert sha(source)==plan['source_sha256'];rgba=np.array(Image.open(source));h,w=rgba.shape[:2];ox,oy=plan['source_top_left'];front=np.array(plan['front']);centers=front-ray*.6;attach=next(a for a in attachments['states'] if a['state']==state);chains=guides+[centers[c] for c in plan['segments']]+[np.array(attach['climber_join']),np.array(attach['right_branch'])]
  bpy.ops.wm.read_factory_settings(use_empty=True);scene=bpy.context.scene;scene.render.threads_mode='FIXED';scene.render.threads=2;obj=bpy.data.objects.new('Hidden archer05 '+state+' supported climbing shrub',bpy.data.meshes.new('Sparse climbing foliage'));scene.collection.objects.link(obj);asset='croisement02-hidden-archer-05-'+state;obj['asset_group']=asset
  mats=[material('Exact native leaf front',source,True),material('Unknown inferred reverse',source,False),material('Projected source on inferred supporting wood',source,False)]
  for m in mats:
   one_sided(m)
   for node in m.node_tree.nodes:
    if node.type=='TEX_IMAGE':node.extension='CLIP'
  hidden=mats[1];shader=next(n for n in hidden.node_tree.nodes if n.type=='BSDF_PRINCIPLED')
  for key in ['Base Color','Emission Color']:
   for link in list(shader.inputs[key].links):hidden.node_tree.links.remove(link)
   shader.inputs[key].default_value=(.35,.35,.35,1)
  vertices=[];faces=[];uvs=[];slots=[];known=[];leaf_checks=[];failed_front=[];front_corrections=[]
  def add(points,slot,observed=False):
   off=len(vertices);vertices.extend(np.asarray(points).tolist());uvs.extend([((p[0]-ox)/w,1-(-p[1]*SIN-p[2]*COS-oy)/h) for p in points]);faces.append(tuple(range(off,off+len(points))));slots.append(slot);known.append(observed)
  def leaf(points,observed):
   points=np.asarray(points)
   if np.cross(points[1]-points[0],points[2]-points[0])@ray<0:points=points[::-1]
   add(points,0 if observed else 2,observed);add(points[::-1]-ray*.015,1,False)
  def world(sx,sy):
   z=float(map_coordinates(field,[[sy-origin[1]-.5],[sx-origin[0]-.5]],order=1,mode='nearest')[0])+1.2*SIN
   return np.array([sx,-(sy+z*COS)/SIN,z])
  for y,x in np.argwhere(rgba[:,:,3]>=128):
   p=np.array([world(ox+x,oy+y),world(ox+x+1,oy+y),world(ox+x+1,oy+y+1),world(ox+x,oy+y+1)]);ok,minimum=clear_polygon(p);shift=0.
   while not ok and shift<6:
    p+=ray*.5;shift+=.5;ok,minimum=clear_polygon(p)
   if shift:front_corrections.append([int(x+ox),int(y+oy),shift])
   leaf_checks.append(minimum)
   if not ok:failed_front.append([int(x+ox),int(y+oy),float(minimum)])
   leaf(p,True)
  # Tapered closed stems retain the previously cleared centerlines/radius caps.
  for ci,chain in enumerate(chains):
   base_radius=.65 if ci<3 or ci==len(chains)-1 else .18
   for si,(a,b) in enumerate(zip(chain[:-1],chain[1:])):
    axis=Vector(b-a)
    if axis.length<1e-7:continue
    axis.normalize();u=axis.cross(Vector((0,0,1)))
    if u.length<.01:u=axis.cross(Vector((1,0,0)))
    u.normalize();v=axis.cross(u);rings=[]
    for endpoint,p in enumerate([a,b]):
     radius=base_radius*(1-.35*(si+endpoint)/max(1,len(chain)-1));rings.append(np.array([p+radius*np.array(u*math.cos(j*math.tau/8)+v*math.sin(j*math.tau/8)) for j in range(8)]))
    polys=[rings[0][::-1],rings[1]]+[np.array([rings[0][j],rings[0][(j+1)%8],rings[1][(j+1)%8],rings[1][j]]) for j in range(8)]
    for polygon in polys:
     normal=np.cross(polygon[1]-polygon[0],polygon[2]-polygon[0]);add(polygon,2 if normal@ray>0 else 1,False)
  # Compact individual oval leaves cluster around existing supported branches.
  rng=np.random.default_rng(505+(state=='applied'));branch_points=np.vstack(chains);branch_tree=cKDTree(branch_points);dist,_=branch_tree.query(front);eligible=np.where(dist<8)[0];accepted=0;rejected=0;interior_bounds=[]
  for attempt in range(450 if state=='initial' else 340):
   index=int(rng.choice(eligible));center=front[index]-ray*float(rng.uniform(.5,2.4));normal=rng.normal(size=3);normal/=np.linalg.norm(normal);u=np.cross(normal,[0,0,1]);u/=max(np.linalg.norm(u),1e-9);v=np.cross(normal,u);radius=float(rng.uniform(1.1,2.6));points=np.array([center+radius*(u*math.cos(j*math.tau/8)+v*.65*math.sin(j*math.tau/8)) for j in range(8)])
   # Keep inferred foliage behind every source-facing envelope point it projects to.
   projected=np.column_stack([points[:,0],-points[:,1]*SIN-points[:,2]*COS]);envelope_points=np.array([world(x,y) for x,y in projected]);retreat=max(0,float(((points-envelope_points)@ray).max())+.4);points-=ray*retreat;ok,minimum=clear_polygon(points)
   if not ok:rejected+=1;continue
   leaf(points,False);accepted+=1;interior_bounds.append(minimum)
  shape=replace_mesh(obj,vertices,faces,uvs,mats,slots,known);obj['foliage_physical_opacity']=True;obj['state_endpoint']=state
  native,_,_=_tree([obj]);missing=[];extra=[];mismatches=[]
  for y in range(-2,h+2):
   for x in range(-2,w+2):
    start=Vector((ox+x+.5,-(oy+y+.5)/SIN,0))+RAY*6000;p,_,_,_=native.ray_cast(start,-RAY);expected=0<=x<w and 0<=y<h and rgba[y,x,3]>=128
    if expected and p is None:missing.append([x+ox,y+oy])
    if not expected and p is not None and y+oy>=0:extra.append([x+ox,y+oy])
  passed=not failed_front and not missing and not extra;all_pass &= passed;budget(1048576);write_json(folder/'pre-save-guard.json',dict(status='PASS' if passed else 'HOLD',native_expected=plan['native_centers'],missing=missing,extra=extra,front_leaf_failures=failed_front,local_front_ray_corrections=front_corrections,minimum_native_leaf_clearance_lower_bound=min(leaf_checks),interior_leaves=accepted,rejected_interior_leaves=rejected,minimum_interior_leaf_clearance_lower_bound=min(interior_bounds) if interior_bounds else None,stem_guard_sha256=sha(guard),limits=['Native RGB/UV shader proof is performed after saved-model reopen, before review handoff.','Inferred stems use explicit native endpoint opacity; solid connectivity does not imply every stem is visible in every view.','No flat cut across off-map top is accepted as final completion; current out-of-map leaf extension remains pending.']))
  if not passed:continue
  budget(16*1024**2);bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(folder/'model.blend'),compress=True);assert (folder/'model.blend').stat().st_size<=16*1024**2;budget();write_json(folder/'construction.json',dict(asset_id=asset,model_sha256=sha(folder/'model.blend'),source=str(source),source_sha256=sha(source),source_top_left=[ox,oy],source_opaque_centers=plan['native_centers'],shape=shape,status='Private sparse climbing candidate; final visual/known-UV review pending',stem_guard_sha256=sha(guard),plan_sha256=sha(planpath),leaf_guard_sha256=sha(folder/'pre-save-guard.json'),interior_leaf_pairs=accepted,limitations=['Explicit inferred right shrub and side extension; native color does not prove hidden wood color.','Source opacity on inferred support can leave hidden portions invisible; evaluate actual/solid side morphology.','Off-map top continuation pending; no geometry-ready claim.']))
 if not all_pass:print('HOLD: actual leaf or native guard failed; no all-eight views',flush=True);return
 from restart14_hidden_archer_review_v12 import main as review
 review(DEST)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
