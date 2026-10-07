"""Bounded private climbing support with explicit native-ray constraints."""
import sys,json,math,shutil,os
from pathlib import Path
os.environ['OPENBLAS_NUM_THREADS']='2'
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from scipy.sparse import coo_matrix
from scipy.sparse.csgraph import dijkstra,connected_components
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from tree_geometry import RAY,SIN
from refinement_review import _tree
from evidence_io import sha,write_json
from render_slots import acquire,release
DEST=OUT/'restart14-hidden-archer/climbing-v7'
def budget(extra=0):
 assert shutil.disk_usage(OUT).free-extra>=10*1024**3,'Hard free-space floor reached'
 used=sum(p.stat().st_size for p in DEST.rglob('*') if p.is_file()) if DEST.exists() else 0
 assert used+extra<=128*1024**2,'Whole-output cap reached'
def main():
 budget();DEST.mkdir(exist_ok=True);assert not any(p.is_file() for p in DEST.rglob("*")), "Nonempty trial must remain immutable";all_pass=True
 for state in ['initial','applied']:
  old=OUT/f'restart14-hidden-archer/candidate-v3/profile-05-{state}';folder=DEST/old.name;budget();folder.mkdir(exist_ok=True);r=json.loads((old/'construction.json').read_text());assert sha(old/'model.blend')==r['model_sha256'];bpy.ops.wm.open_mainfile(filepath=str(old/'model.blend'));scene=bpy.context.scene;scene.render.threads_mode='FIXED';scene.render.threads=2;leaf=next(o for o in scene.objects if o.type=='MESH');tree,_,_=_tree([leaf]);rgba=np.array(Image.open(r['source']));alpha=rgba[:,:,3]>=128;yy,xx=np.where(alpha);x0,y0=r['source_top_left'];native=[];origins=[]
  for y,x in zip(yy,xx):
   origin=Vector((x0+x+.5,-(y0+y+.5)/SIN,0))+RAY*6000;p,_,_,_=tree.ray_cast(origin,-RAY);assert p is not None;native.append(np.array(p));origins.append(origin)
  native=np.array(native);ray=np.array(RAY);points=native-ray*1.0;lookup={(int(x),int(y)):i for i,(x,y) in enumerate(zip(xx,yy))};rows=[];cols=[];weights=[]
  for (x,y),i in lookup.items():
   for dx,dy in [(1,0),(0,1),(1,1),(-1,1)]:
    j=lookup.get((x+dx,y+dy))
    if j is not None:rows.extend([i,j]);cols.extend([j,i]);v=np.linalg.norm(points[i]-points[j]);weights.extend([v,v])
  graph=coo_matrix((weights,(rows,cols)),shape=(len(points),len(points))).tocsr();_,labels=connected_components(graph);largest=int(np.argmax(np.bincount(labels)));domain=np.where(labels==largest)[0];root=int(domain[np.argmin(points[domain,2])]);_,pred=dijkstra(graph,indices=root,return_predecessors=True)
  # Every pixel in the main connected leaf mass receives a physical twig path.
  edges=[(int(i),int(pred[i])) for i in domain if i!=root];anchor=points[root].copy();anchor[2]=43.95;anchor_index=len(points);points=np.vstack([points,anchor]);edges.append((root,anchor_index));offsets=np.zeros(len(points));radii=np.full(len(edges),.25);radii[-1]=.35;trace=[]
  def geometry():
   vertices=[];faces=[];owners=[];moved=points-offsets[:,None]*ray
   for ei,(i,j) in enumerate(edges):
    a=Vector(moved[i]);b=Vector(moved[j]);axis=b-a
    if axis.length<1e-6:continue
    axis.normalize();u=axis.cross(Vector((0,0,1)))
    if u.length<.01:u=axis.cross(Vector((1,0,0)))
    u.normalize();v=axis.cross(u);off=len(vertices);n=6
    for p in [a,b]:
     for k in range(n):vertices.append(tuple(p+float(radii[ei])*(u*math.cos(k*math.tau/n)+v*math.sin(k*math.tau/n))))
    polygon=[tuple(off+k for k in reversed(range(n))),tuple(off+n+k for k in range(n))]
    polygon += [(off+k,off+(k+1)%n,off+n+(k+1)%n,off+n+k) for k in range(n)]
    for f in polygon:
     for k in range(1,len(f)-1):faces.append((f[0],f[k],f[k+1]));owners.append(ei)
   return vertices,faces,owners
  for iteration in range(16):
   vertices,faces,owners=geometry();branch=BVHTree.FromPolygons(vertices,faces,all_triangles=True);violations=[]
   for k,origin in enumerate(origins):
    p,_,tri,_=branch.ray_cast(origin,-RAY)
    if p is not None:
     advance=float((np.array(p)-native[k])@ray)
     if advance>=-.02:violations.append((k,owners[tri],advance+.3))
   trace.append(dict(iteration=iteration,known_ray_interference=len(violations),maximum_node_retreat=float(offsets.max()),minimum_support_z=float((points-offsets[:,None]*ray)[:,2].min())))
   print(state,trace[-1],flush=True)
   if not violations:break
   desired=offsets.copy()
   for _,edge,amount in violations:
    for node in edges[edge]:
     if node!=anchor_index:desired[node]=max(desired[node],offsets[node]+amount)
   # Preserve a shared branch junction at every graph vertex. Do not shift the
   # rooted anchor or bury the stem network to hide visibility failures.
   max_retreat=np.maximum(0,(points[:,2]-43.8)/SIN);desired=np.minimum(desired,max_retreat);desired[anchor_index]=0
   if np.max(desired-offsets)<1e-5:break
   offsets=desired
  vertices,faces,owners=geometry();branch=BVHTree.FromPolygons(vertices,faces,all_triangles=True);violations=[];extras=[]
  for y in range(-2,alpha.shape[0]+2):
   for x in range(-2,alpha.shape[1]+2):
    origin=Vector((x0+x+.5,-(y0+y+.5)/SIN,0))+RAY*6000;p,_,_,_=branch.ray_cast(origin,-RAY)
    if p is None:continue
    k=lookup.get((x,y))
    if k is None:extras.append([x0+x,y0+y])
    elif float((np.array(p)-native[k])@ray)>=-.02:violations.append([x0+x,y0+y])
  passed=not violations and not extras;all_pass &= passed;budget();write_json(folder/'native-support-guard.json',dict(status='PASS' if passed else 'HOLD',parent_sha256=r['model_sha256'],iterations=trace,known_blocked=violations,extra_source_centers=extras,main_component_pixels=len(domain),source_pixels=len(native),supported_graph_edges=len(edges),anchor_world=anchor.tolist(),minimum_stem_center_z=float((points-offsets[:,None]*ray)[:,2].min()),limits=['Finite native-ray guard only; actual rock-surface intersections and complete side-view coherence still need independent checks.']))
  if not passed:continue
  mesh=bpy.data.meshes.new('Connected inferred climbing branches');mesh.from_pydata(vertices,[],faces);mesh.update();obj=bpy.data.objects.new('Protected rooted support network',mesh);scene.collection.objects.link(obj);mat=bpy.data.materials.new('Unreviewed hidden branch appearance');mat.use_nodes=True;mat.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value=(.35,.35,.35,1);mesh.materials.append(mat);bpy.ops.object.select_all(action='DESELECT');leaf.select_set(True);obj.select_set(True);bpy.context.view_layer.objects.active=leaf;bpy.ops.object.join();budget(16*1024**2);bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(folder/'model.blend'),compress=True);assert (folder/'model.blend').stat().st_size<=16*1024**2;budget();write_json(folder/'construction.json',{**r,'model_sha256':sha(folder/'model.blend'),'parent_model':str(old/'model.blend'),'parent_sha256':r['model_sha256'],'status':'Private supported climbing volume; physical contact and visual review pending','source_protection_guard_sha256':sha(folder/'native-support-guard.json'),'support_edges':len(edges),'main_component_pixels':len(domain)})
 if not all_pass:
  print('STOP: native guard failed; no all-eight renders authorized for this trial',flush=True);return
 # Render only after both states pass the hard native guard.
 from restart14_hidden_archer_review import main as review
 os.environ['HIDDEN_ARCHER_SMALL_BUILD']='1';review(DEST)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
