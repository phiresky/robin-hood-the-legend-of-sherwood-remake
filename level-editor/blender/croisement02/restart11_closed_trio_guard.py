"""Reopen the limited support trio and verify exact visible source and closed coverage."""
from pathlib import Path
import sys,json,hashlib,io
import bpy,bmesh,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.geometry import barycentric_transform
P=Path(__file__).resolve().parent;sys.path.insert(0,str(P))
from restart6_source_gap_audit import OUT,RAY,SIN
from refinement_review import _tree
from render_slots import acquire,release
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 worker=OUT/'restart11-hiding-mound/closed-support-trio-v1';r=json.loads((worker/'validation.json').read_text());model=worker/'model.blend';assert sha(model)==r['model_sha256'];src=json.loads((OUT/'restart9-hiding-scatter/mound-flat-v2/validation.json').read_text());rgba=np.array(Image.open(src['source']).convert('RGBA'));h,w=rgba.shape[:2];bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update();rows=[]
 for row in r['records']:
  objects=[bpy.data.objects[n]for n in row['objects']];tree,_,_=_tree(objects);triangles=[];colors={};topology=[];x0,y0=row['source_origin']
  for obj in objects:
   obj.data.calc_loop_triangles();triangles.extend((obj,t)for t in obj.data.loop_triangles);bm=bmesh.new();bm.from_mesh(obj.data);topology.append(dict(object=obj.name,nonmanifold=sum(not e.is_manifold for e in bm.edges),volume=bm.calc_volume(signed=True)));bm.free()
   for i,m in enumerate(obj.data.materials):
    tex=next(n for n in m.node_tree.nodes if n.type=='TEX_IMAGE');colors[(obj.name,i)]=np.array(Image.open(io.BytesIO(bytes(tex.image.packed_file.data))).convert('RGBA'));shader=next(n for n in m.node_tree.nodes if n.type=='BSDF_PRINCIPLED');assert not shader.inputs['Alpha'].links and shader.inputs['Alpha'].default_value==1
  missing=[];foreign=[];wrong=[];maximum=0.;tested=0
  for y in range(h):
   for x in range(w):
    p,_,ti,_=tree.ray_cast(Vector((float(x0+x+.5),float(-(y0+y+.5)/SIN),0))+RAY*2000,-RAY)
    if rgba[y,x,3]==0:
     if p is not None:foreign.append([x,y])
     continue
    if p is None:missing.append([x,y]);continue
    obj,t=triangles[ti];q=barycentric_transform(p,*[obj.matrix_world@obj.data.vertices[i].co for i in t.vertices],*[Vector((*obj.data.uv_layers.active.data[i].uv,0))for i in t.loops]);sx=q.x*w;sy=(1-q.y)*h;maximum=max(maximum,abs(sx-x-.5),abs(sy-y-.5));actual=colors[(obj.name,t.material_index)][int(sy),int(sx),:3]
    if t.material_index!=0 or not np.array_equal(actual,rgba[y,x,:3]):wrong.append(dict(pixel=[x,y],material=t.material_index,rgb=actual.tolist(),expected=rgba[y,x,:3].tolist()))
    tested+=1
  valid=not(missing or foreign or wrong)and tested==1011 and maximum<.01 and all(t['nonmanifold']==0 and t['volume']>0 for t in topology);rows.append(dict(tag=row['tag'],status='PASS'if valid else'HOLD',opaque_centers=tested,missing=missing,foreign=foreign,wrong=wrong,maximum_texel_error=maximum,topology=topology))
 (worker/'saved-native-guard.json').write_text(json.dumps(dict(status='PASS'if all(x['status']=='PASS'for x in rows)else'HOLD',model_sha256=sha(model),source_sha256=src['source_sha256'],records=rows,scope='Three reopened source RGB/topology/actual opaque coverage guards. Support extrema are separately bound by the construction receipt; context remains required before approval.'),indent=2)+'\n');print(json.dumps([dict(tag=x['tag'],status=x['status'],wrong=len(x['wrong']))for x in rows]))
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
