"""Private rooted climbing scaffold following source-connected foliage paths."""
import sys,json,math
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from scipy.sparse import coo_matrix
from scipy.sparse.csgraph import dijkstra,connected_components
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from tree_geometry import RAY,SIN
from refinement_review import _tree
from evidence_io import sha,write_json
from render_slots import acquire,release

def main():
 base=OUT/'restart14-hidden-archer/candidate-v3';dest=OUT/'restart14-hidden-archer/climbing-v6';dest.mkdir(exist_ok=False)
 for state in ['initial','applied']:
  old=base/f'profile-05-{state}';folder=dest/old.name;folder.mkdir();r=json.loads((old/'construction.json').read_text());assert sha(old/'model.blend')==r['model_sha256'];bpy.ops.wm.open_mainfile(filepath=str(old/'model.blend'));scene=bpy.context.scene;leaf=next(o for o in scene.objects if o.type=='MESH');tree,_,_=_tree([leaf]);rgba=np.array(Image.open(r['source']));alpha=rgba[:,:,3]>=128;yy,xx=np.where(alpha);x0,y0=r['source_top_left'];points=[]
  for y,x in zip(yy,xx):
   p,_,_,_=tree.ray_cast(Vector((x0+x+.5,-(y0+y+.5)/SIN,0))+RAY*6000,-RAY);assert p is not None;points.append(np.array(p)-np.array(RAY)*1.0)
  points=np.array(points);lookup={(int(x),int(y)):i for i,(x,y) in enumerate(zip(xx,yy))};rows=[];cols=[];weights=[]
  for (x,y),i in lookup.items():
   for dx,dy in [(1,0),(0,1),(1,1),(-1,1)]:
    j=lookup.get((x+dx,y+dy))
    if j is not None:rows.extend([i,j]);cols.extend([j,i]);v=np.linalg.norm(points[i]-points[j]);weights.extend([v,v])
  graph=coo_matrix((weights,(rows,cols)),shape=(len(points),len(points))).tocsr();_,labels=connected_components(graph);largest=int(np.argmax(np.bincount(labels)));domain=np.where(labels==largest)[0];root=int(domain[np.argmin(points[domain,2])]);dist,pred=dijkstra(graph,indices=root,return_predecessors=True);targets=[]
  for coordinate in [0,1,2]:
   targets.extend([int(domain[np.argmin(points[domain,coordinate])]),int(domain[np.argmax(points[domain,coordinate])])])
  # Additional supported shoots distributed through the main leaf mass.
  for q in np.linspace(0,len(domain)-1,12).astype(int):targets.append(int(domain[q]))
  edges=set()
  for target in targets:
   node=target
   while node!=root:
    parent=int(pred[node]);assert parent>=0;edges.add(tuple(sorted((node,parent))));node=parent
  vertices=[];faces=[]
  def segment(a,b,radius):
   a=Vector(a);b=Vector(b);axis=b-a
   if axis.length<1e-5:return
   axis.normalize();u=axis.cross(Vector((0,0,1)))
   if u.length<.01:u=axis.cross(Vector((1,0,0)))
   u.normalize();v=axis.cross(u);off=len(vertices);n=6
   for p in [a,b]:
    for k in range(n):vertices.append(tuple(p+radius*(u*math.cos(k*math.tau/n)+v*math.sin(k*math.tau/n))))
   faces.append(tuple(off+k for k in reversed(range(n))));faces.append(tuple(off+n+k for k in range(n)))
   for k in range(n):faces.append((off+k,off+(k+1)%n,off+n+(k+1)%n,off+n+k))
  for i,j in sorted(edges):segment(points[i],points[j],.18)
  anchor=points[root].copy();anchor[2]=43.95;segment(anchor,points[root],.35)
  mesh=bpy.data.meshes.new('Explicit inferred climbing support');mesh.from_pydata(vertices,[],faces);mesh.update();obj=bpy.data.objects.new('Inferred rooted climbing branches',mesh);scene.collection.objects.link(obj);mat=bpy.data.materials.new('Unknown climbing branch appearance');mat.diffuse_color=(.35,.35,.35,1);mat.use_nodes=True;mesh.materials.append(mat)
  bpy.ops.object.select_all(action='DESELECT');leaf.select_set(True);obj.select_set(True);bpy.context.view_layer.objects.active=leaf;bpy.ops.object.join();bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(folder/'model.blend'),compress=True)
  write_json(folder/'construction.json',{**r,'model_sha256':sha(folder/'model.blend'),'parent_model':str(old/'model.blend'),'parent_sha256':r['model_sha256'],'status':'Private climbing support trial; native and rock-intersection review required','scaffold_edges':len(edges),'scaffold_radius':.18,'root_world':anchor.tolist(),'rooted_component_pixels':len(domain),'unconnected_source_components':int(len(points)-len(domain)),'limits':['New hidden branches are explicit geometric inference, not source-observed wood.','Neighbor intersections, source extras and oblique coherence remain unverified.']})
 from restart14_hidden_archer_review import main as review
 review(dest)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
