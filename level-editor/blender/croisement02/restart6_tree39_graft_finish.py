"""Correct Boolean material ownership and six bounded native toe centers."""
import sys,json,hashlib
from pathlib import Path
import bpy,bmesh,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from mathutils.geometry import barycentric_transform
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
from restart6_tree39_contour import ROOT,OUT,SPECS,RAY,SIN,COS,covered,projected_tree,projection
from render_slots import acquire,release
from evidence_io import sha,write_json

def main():
 out=ROOT/'tree39-continuous-graft-v3';out.mkdir(exist_ok=False);source=SPECS[39][0];parent=ROOT/'tree39-continuous-graft-v2/model.blend';bpy.ops.wm.open_mainfile(filepath=str(source));bpy.context.view_layer.update();wood=[o for o in bpy.context.scene.objects if o.type=='MESH'and o.get('asset_group')=='croisement02-tree-39'and'Crown'not in o.name];original=next(o for o in wood if o.get('source_node')=='building-095');material_name=original.data.materials[2].name;surface=BVHTree.FromPolygons([original.matrix_world@v.co for v in original.data.vertices],[list(f.vertices)for f in original.data.polygons]);item=next(r for r in json.load(open(OUT/'review-mask-inventory.json'))['masks']if r['index']==39);ox,oy=item['box_top_left'];w,h=item['box_size'];yy,xx=np.where(np.array(Image.open(item['png']))>0);pts=np.column_stack((xx+ox+.5,yy+oy+.5));oldcov=covered(wood,pts);gap=np.array(Image.open(ROOT/'source-audit-v1/exposed-39.png'))>0;targets=pts[oldcov|gap[yy+oy,xx+ox]];bpy.ops.wm.open_mainfile(filepath=str(parent));scene=bpy.context.scene;bpy.context.view_layer.update();wood=[o for o in scene.objects if o.type=='MESH'and'Crown'not in o.name];existing={hashlib.sha256(bytes(im.packed_file.data)).hexdigest():im for im in bpy.data.images if im.packed_file}
 with bpy.data.libraries.load(str(source),link=False)as(a,b):b.materials=[material_name]
 restored=b.materials[0]
 for node in restored.node_tree.nodes:
  if node.type=='TEX_IMAGE'and node.image and node.image.packed_file:
   digest=hashlib.sha256(bytes(node.image.packed_file.data)).hexdigest()
   if digest in existing:node.image=existing[digest]
 upper=next(o for o in wood if o.get('source_node')=='building-095');upper.data.materials.append(restored);slot=len(upper.data.materials)-1;restored_faces=0
 for f in upper.data.polygons:
  if f.center.z<99:continue
  if all(surface.find_nearest(upper.matrix_world@upper.data.vertices[i].co)[3]<.003 for i in f.vertices):f.material_index=slot;restored_faces+=1
 lower=next(o for o in wood if o.get('source_node')=='building-096');before=np.array([v.co[:]for v in lower.data.vertices]);history=[]
 for iteration in range(12):
  cov=covered(wood,targets);history.append(dict(iteration=iteration,covered=int(cov.sum()),required=len(targets)));print(history[-1],flush=True)
  if cov.all():break
  tree,verts,world,faces,owners=projected_tree([lower]);p=np.array([v.co[:]for v in lower.data.vertices]);q=projection(p);sums=np.zeros((len(p),2));weights=np.zeros(len(p))
  for target in targets[~cov]:
   hit,n,i,d=tree.find_nearest(Vector((*target,0)));delta=target-np.array(hit[:2]);distance=np.linalg.norm(delta)
   if distance<1e-4:continue
   delta*=1+.65/distance;radius=max(4.,2*distance+3);dq=np.linalg.norm(q-np.array(hit[:2]),axis=1);weight=np.exp(-2*(dq/radius)**2);weight[(dq>radius*2.5)|(p[:,2]>12)]=0;sums+=weight[:,None]*delta;weights+=weight
  shift=sums/(1+weights[:,None]);p+=np.column_stack((shift[:,0],-SIN*shift[:,1],-COS*shift[:,1]));low=p[:,2]<.1501;p[low,1]-=(.1501-p[low,2])*COS/SIN;p[low,2]=.1501
  for v,point in zip(lower.data.vertices,p):v.co=point
  lower.data.update();bpy.context.view_layer.update()
 cleanup={}
 for o in wood:
  bm=bmesh.new();bm.from_mesh(o.data);n=0
  for _ in range(100):
   faces=[f for f in bm.faces if f.calc_area()<1e-9 and max(v.co.z for v in f.verts)<110]
   if not faces:break
   edge=min(faces[0].edges,key=lambda e:e.calc_length())
   if edge.calc_length()>.01:raise ValueError(('Unbounded degenerate cleanup',o.name,edge.calc_length()))
   bmesh.ops.collapse(bm,edges=[edge],uvs=False);n+=1
  bm.to_mesh(o.data);bm.free();o.data.update();cleanup[o.name]=n
  uv=o.data.uv_layers['Continuous lower native projection']
  for li,loop in enumerate(o.data.loops):
   v=o.matrix_world@o.data.vertices[loop.vertex_index].co;uv.data[li].uv=((v.x-ox)/w,1-(-v.y*SIN-v.z*COS-oy)/h)
 bpy.context.view_layer.update();cov=covered(wood,targets);assert cov.all(),(targets[~cov]-.5).tolist();bpy.ops.outliner.orphans_purge(do_recursive=True);bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'),compress=True);write_json(out/'finish.json',dict(model_sha256=sha(out/'model.blend'),parent_sha256=sha(parent),approved_source_sha256=sha(source),restored_upper_material=material_name,restored_upper_faces=restored_faces,native_history=history,max_toe_movement=float(np.linalg.norm(p-before,axis=1).max()),degenerate_cleanup=cleanup,scope='Original upper material restored explicitly; six toe centers repaired belowZ12, native UV recomputed after bounded lower cleanup. No crown/support geometry change.'))
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
