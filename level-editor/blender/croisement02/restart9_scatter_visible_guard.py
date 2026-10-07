"""Check saved scatter texels and reject duplicate painted depth layers."""
from pathlib import Path
import sys,json,hashlib
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from mathutils.geometry import barycentric_transform
P=Path(__file__).resolve().parent;sys.path.insert(0,str(P))
from restart6_source_gap_audit import OUT,RAY,SIN
from render_slots import acquire,release
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 worker=OUT/'restart9-hiding-scatter/scatter-surfaces-v2';out=worker/'saved-pixel-guard-v1';out.mkdir(exist_ok=False);rec=json.loads((worker/'manifest.json').read_text());model=worker/'model.blend';assert sha(model)==rec['model_sha256'];bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update();results=[];failures=[]
 for row in rec['records']:
  source=np.array(Image.open(row['source']).convert('RGBA'));h,w=source.shape[:2];vertices=[];faces=[];uvs=[];images=[];faceimages=[]
  for oi,name in enumerate(row['objects']):
   obj=bpy.data.objects[name];mesh=obj.data;mesh.calc_loop_triangles();start=len(vertices);vertices.extend([obj.matrix_world@v.co for v in mesh.vertices]);image=next(n.image for n in mesh.materials[0].node_tree.nodes if n.type=='TEX_IMAGE');pin=row['receivers'][oi];assert hashlib.sha256(bytes(image.packed_file.data)).hexdigest()==pin['texture_sha256'];images.append(np.array(Image.open(worker/pin['texture']).convert('RGBA')))
   for t in mesh.loop_triangles:faces.append(tuple(start+i for i in t.vertices));uvs.append([Vector((*mesh.uv_layers.active.data[i].uv,0))for i in t.loops]);faceimages.append(oi)
  tree=BVHTree.FromPolygons(vertices,faces,all_triangles=True);missing=[];duplicate=[];wrong=[];x0,y0=row['bbox'][:2]
  for y,x in np.argwhere(source[:,:,3]>0):
   origin=Vector((x0+x+.5,-(y0+y+.5)/SIN,0))+RAY*6000;seen=[]
   for attempt in range(64):
    p,n,i,d=tree.ray_cast(origin,-RAY)
    if p is None:break
    q=barycentric_transform(p,*[vertices[v]for v in faces[i]],*uvs[i]);tx=min(w-1,max(0,int(q.x*w)));ty=min(h-1,max(0,int((1-q.y)*h)));rgba=images[faceimages[i]][ty,tx]
    if rgba[3]>0 and (not seen or (p-seen[-1]).length>.01):
     seen.append(p.copy())
     if not np.array_equal(rgba,source[y,x]):wrong.append([int(x),int(y)])
    origin=p-RAY*.01
   else:raise RuntimeError('Unbounded receiver intersections')
   if not seen:missing.append([int(x),int(y)])
   if len(seen)>1:duplicate.append(dict(pixel=[int(x),int(y)],layers=len(seen)))
  result=dict(index=row['index'],instances=row['instances'],source_pixels=row['source_pixels'],missing=missing,wrong_RGBA=wrong,duplicate_painted_depth_layers=duplicate);results.append(result)
  if missing or wrong or duplicate:failures.append(row['index'])
 (out/'report.json').write_text(json.dumps(dict(status='HOLD'if failures else 'PASS',model_sha256=sha(model),failed_endpoints=failures,records=results),indent=2)+'\n');print('FAILED',failures,flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
