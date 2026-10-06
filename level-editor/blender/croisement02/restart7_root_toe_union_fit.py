"""Restore prior native silhouette after continuous lower construction."""
import sys,json
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.geometry import barycentric_transform
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
from restart6_tree38_contour import ROOT,OUT,RAY,SIN,COS,covered,projected_tree,projection
from evidence_io import sha,write_json
from render_slots import acquire,release

def main(number):
 parent=ROOT/f'tree{number}-toe-union-v{7 if number==19 else 8}';out=ROOT/f'tree{number}-toe-union-fit-v9';out.mkdir(exist_ok=False);prior=json.load(open(ROOT/f'baseline-audit-{number}-v3/report.json'));source=Path(prior['source']);asset=f'croisement02-tree-{number}';item=next(x for x in json.load(open(OUT/'review-mask-inventory.json'))['masks']if x['index']==number);ox,oy=item['box_top_left'];w,h=item['box_size'];yy,xx=np.where(np.array(Image.open(item['png']))>0);alltargets=np.column_stack((xx+ox+.5,yy+oy+.5));bpy.ops.wm.open_mainfile(filepath=str(source));bpy.context.view_layer.update();wood=[o for o in bpy.context.scene.objects if o.type=='MESH'and o.get('asset_group')==asset and o.get('projection_component')!='crown'];old=covered(wood,alltargets);targets=alltargets[old];bpy.ops.wm.open_mainfile(filepath=str(parent/'model.blend'));bpy.context.view_layer.update();wood=[o for o in bpy.context.scene.objects if o.type=='MESH'and o.get('asset_group')==asset and o.get('projection_component')!='crown'];before={o.name:np.array([o.matrix_world@v.co for v in o.data.vertices])for o in wood};limit=43 if number==19 else 71;history=[]
 for iteration in range(16):
  cov=covered(wood,targets);history.append(dict(iteration=iteration,hits=int(cov.sum()),total=len(cov)));print(history[-1],flush=True)
  if cov.all():break
  tree,verts,world,faces,owners=projected_tree(wood);constraints=[]
  for q in targets[~cov]:
   p,n,i,d=tree.find_nearest(Vector((*q,0)));ids=faces[i];anchor=barycentric_transform(p,*[verts[j]for j in ids],*[world[j]for j in ids]);delta=q-np.array(p[:2]);dist=np.linalg.norm(delta)
   if dist<1e-5:continue
   constraints.append((np.array(p[:2]),np.array(anchor),delta*(1+.5/dist),float(d)))
  for obj in wood:
   a=np.array([obj.matrix_world@v.co for v in obj.data.vertices]);ids=np.flatnonzero(a[:,2]<limit);p=a[ids];q=projection(p);sums=np.zeros((len(ids),2));weights=np.zeros(len(ids))
   for center,anchor,delta,dist in constraints:
    radius=max(2.5,dist*2+2);dq=np.linalg.norm(q-center,axis=1);depth=abs((p-anchor)@np.array(RAY));weight=np.exp(-2*(dq/radius)**2-2*(depth/(radius+4))**2);weight[(dq>radius*1.5)|(depth>radius+8)]=0;sums+=weight[:,None]*delta;weights+=weight
   shift=sums/(1+weights[:,None]);p+=np.column_stack((shift[:,0],-SIN*shift[:,1],-COS*shift[:,1]));low=p[:,2]<.15;p[low,1]-=(.15-p[low,2])*COS/SIN;p[low,2]=.15;inv=obj.matrix_world.inverted()
   for i,v in zip(ids,p):obj.data.vertices[int(i)].co=inv@Vector(v)
   obj.data.update()
  bpy.context.view_layer.update()
 cov=covered(wood,targets);movement={}
 for o in wood:
  p=np.array([o.matrix_world@v.co for v in o.data.vertices]);delta=np.linalg.norm(p-before[o.name],axis=1);assert np.all(delta[before[o.name][:,2]>=limit]==0);movement[o.name]=float(delta.max());uv=o.data.uv_layers['Continuous toe native projection']
  for li,loop in enumerate(o.data.loops):
   v=o.matrix_world@o.data.vertices[loop.vertex_index].co;uv.data[li].uv=((v.x-ox)/w,1-(-v.y*SIN-v.z*COS-oy)/h)
 write_json(out/'native-fit.json',dict(parent_sha256=sha(parent/'model.blend'),source_sha256=sha(source),history=history,missing=(targets[~cov]-.5).tolist(),movement=movement,upper_unchanged=True,scope='Restores previous native center coverage only; ambiguous residuals not forced to wood.'))
 bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'),compress=True)
if __name__=='__main__':
 acquire()
 try:main(int(sys.argv[sys.argv.index('--')+1]))
 finally:release()
