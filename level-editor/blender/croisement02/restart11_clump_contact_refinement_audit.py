"""Reconcile source-corner support with opaque triangle samples before correction."""
from pathlib import Path
import sys,json,hashlib,io
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
P=Path(__file__).resolve().parent;sys.path.insert(0,str(P))
from restart6_source_gap_audit import OUT,RAY,SIN
from leaf_state_scene_context import load_scene
from restart11_clump_support_trio import rawtree
from render_slots import acquire,release
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def crossings(tree,p):
 direction=Vector((.733,.411,.537)).normalized();origin=p+direction*.0001;distances=[];travel=0.
 for _ in range(100):
  q,_,_,distance=tree.ray_cast(origin,direction)
  if q is None:break
  travel+=distance;distances.append(travel);origin=q+direction*.001;travel+=.001
 else:raise RuntimeError('Unbounded solid parity ray')
 return len(distances)
def main():
 worker=OUT/'restart11-hiding-mound/support-trio-v1';r=json.loads((worker/'validation.json').read_text());model=worker/'model.blend';assert sha(model)==r['model_sha256'];out=worker/'contact-refinement-audit-v1';out.mkdir(exist_ok=False);sourceaudit=json.loads((OUT/'restart9-hiding-scatter/terrain-receivers-v2/report.json').read_text());scene,static,pins,base=load_scene();ground=[o for o in static if o.name.startswith('Croisement02 Terrain')or o.get('asset_group')=='croisement02-north-woodland-bank'];walls=[o for o in static if o.get('asset_group')=='croisement02-southeast-stone-wall-and-gate'];terrain=rawtree(ground);withwall=rawtree(ground+walls);walltree=rawtree(walls);wallparts=[(o.name,rawtree([o]))for o in walls];bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update();records=[]
 for row in r['records']:
  authority=next(q for q in sourceaudit['records']if q['id']==row['instance']);x0,y0=np.array(authority['display_position'])+authority['initial']['offset'];support=withwall if row['tag']=='wall'else terrain
  for name in row['objects']:
   obj=bpy.data.objects[name];image=next(n.image for n in obj.data.materials[0].node_tree.nodes if n.type=='TEX_IMAGE');rgba=np.array(Image.open(io.BytesIO(bytes(image.packed_file.data))).convert('RGBA'));mask=rgba[:,:,3]>0;h,w=mask.shape;tree=rawtree([obj]);nearest=None;newneed=0.;worst=None;inside=[]
   for y,x in np.argwhere(mask):
    for dx,dy in [(a,b)for a in [.005,.5,.995]for b in [.005,.5,.995]]:
     p=Vector((float(x0+x+dx),float(-(y0+y+dy)/SIN),0));back,_,_,_=tree.ray_cast(p-RAY*2000,RAY);front,_,_,_=support.ray_cast(p+RAY*2000,-RAY)
     if back is None:continue
     gap=(back-front).dot(RAY)
     if nearest is None or gap<nearest['ray_gap']:nearest=dict(ray_gap=gap,mound=[*back],receiver=[*front],pixel=[int(x),int(y)],subpixel=[dx,dy])
   obj.data.calc_loop_triangles()
   for tri in obj.data.loop_triangles:
    vs=[obj.matrix_world@obj.data.vertices[i].co for i in tri.vertices];uvs=[obj.data.uv_layers.active.data[i].uv for i in tri.loops]
    for a,b in [(a/8,b/8)for a in range(9)for b in range(9-a)]:
     c=1-a-b;p=vs[0]*a+vs[1]*b+vs[2]*c;uv=uvs[0]*a+uvs[1]*b+uvs[2]*c;x=int(uv.x*w);y=int((1-uv.y)*h)
     if not(0<=x<w and 0<=y<h and mask[y,x]):continue
     receiver,_,_,_=support.ray_cast(p+RAY*2000,-RAY);need=(receiver-p).dot(RAY)+.02
     if need>newneed:newneed=need;worst=dict(mound=[*p],receiver=[*receiver],ray_shift=need,height_gain=need*RAY.z)
     if row['tag']=='wall':
      q,_,_,_=walltree.ray_cast(Vector((p.x,p.y,2000)),Vector((0,0,-1)))
      if q is not None and p.z<q.z-.01:
       parity=[dict(object=n,crossings=crossings(t,p))for n,t in wallparts];inside.append(dict(point=[*p],inside_any=any(v['crossings']%2 for v in parity),parts=parity))
   records.append(dict(tag=row['tag'],object=name,owned=int(mask.sum()),nearest_source_corner_contact=nearest,additional_sampled_ray_shift=max(0,newneed),worst=worst,wall_inside_samples=inside))
 (out/'report.json').write_text(json.dumps(dict(model_sha256=sha(model),records=records,scope='Diagnostic only. Exact source-corner supports and finer opaque-triangle receiver test. Wall parity separates actual solid interiors from overhang footprint.'),indent=2)+'\n');print(json.dumps([dict(object=x['object'],need=x['additional_sampled_ray_shift'],contact=x['nearest_source_corner_contact'],inside=sum(z['inside_any']for z in x['wall_inside_samples']))for x in records]))
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
