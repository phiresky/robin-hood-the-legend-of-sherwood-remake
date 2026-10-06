"""Restore the five previous native edge hits after smoothing the fork union."""
import sys,json
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.geometry import barycentric_transform
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
from restart6_tree24_contour import ROOT,OUT,SIN,COS,RAY,projection,projected_tree,covered
from render_slots import acquire,release
from evidence_io import sha,write_json
acquire()
try:
 source=ROOT/'tree24-fork-union-v5/model.blend';parent=ROOT/'tree24-contour-v2/model.blend';out=ROOT/'tree24-fork-union-v6';out.mkdir(exist_ok=False);item=next(x for x in json.loads((OUT/'review-mask-inventory.json').read_text())['masks']if x['index']==24);ox,oy=item['box_top_left'];y,x=np.where(np.array(Image.open(item['png']))>0);targets=np.column_stack((x+ox+.5,y+oy+.5));bpy.ops.wm.open_mainfile(filepath=str(parent));bpy.context.view_layer.update();wood=[o for o in bpy.context.scene.objects if o.type=='MESH'and'Crown'not in o.name];required=covered(wood,targets);targets=targets[required];bpy.ops.wm.open_mainfile(filepath=str(source));bpy.context.view_layer.update();wood=[o for o in bpy.context.scene.objects if o.type=='MESH'and'Crown'not in o.name];history=[]
 for iteration in range(8):
  cov=covered(wood,targets);history.append(dict(iteration=iteration,covered=int(cov.sum()),total=len(targets)))
  if cov.all():break
  tree,verts,world,faces,owners=projected_tree(wood);constraints=[]
  for q in targets[~cov]:
   point,n,i,dist=tree.find_nearest(Vector((*q,0)));ids=faces[i];anchor=barycentric_transform(point,*[verts[j]for j in ids],*[world[j]for j in ids]);delta=q-np.array(point[:2]);length=np.linalg.norm(delta);constraints.append((np.array(point[:2]),np.array(anchor),delta*(1+.25/max(length,.0001)),owners[i]))
  for obj in wood:
   p=np.array([obj.matrix_world@v.co for v in obj.data.vertices]);q=projection(p);indices=np.flatnonzero((p[:,2]>100)&(p[:,2]<150));a=p[indices];aq=q[indices];sums=np.zeros((len(indices),2));weights=np.zeros(len(indices))
   for center,anchor,delta,owner in constraints:
    if owner!=obj.name:continue
    dq=np.linalg.norm(aq-center,axis=1);depth=abs((a-anchor)@np.array(RAY));w=np.exp(-2*(dq/3.5)**2-2*(depth/10)**2);w[(dq>5)|(depth>15)]=0;sums+=w[:,None]*delta;weights+=w
   shift=sums/(1+weights[:,None]);a+=np.column_stack((shift[:,0],-SIN*shift[:,1],-COS*shift[:,1]));inverse=obj.matrix_world.inverted()
   for j,(i,point)in enumerate(zip(indices,a)):
    if weights[j]>0:obj.data.vertices[int(i)].co=inverse@Vector(point)
   obj.data.update()
  bpy.context.view_layer.update()
 assert covered(wood,targets).all(),history;bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'),compress=True);write_json(out/'native-fit.json',dict(source_sha256=sha(source),model_sha256=sha(out/'model.blend'),history=history,scope='Bounded continuous fork edge fit only, restoring prior native coverage.'))
finally:release()
