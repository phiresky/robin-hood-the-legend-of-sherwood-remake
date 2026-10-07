"""Check native first-hit color and real closed topology independently after reopening."""
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
 name=sys.argv[sys.argv.index('--')+1]if'--'in sys.argv else'closed-small-clumps-v3';assert name.startswith('closed-small-clumps-')and'/'not in name;worker=OUT/'restart11-hiding-mound'/name;r=json.loads((worker/'validation.json').read_text());model=worker/'model.blend';assert sha(model)==r['model_sha256'];s=json.loads((OUT/'restart9-hiding-scatter/mound-flat-v2/validation.json').read_text());rgba=np.array(Image.open(s['source']).convert('RGBA'));h,w=rgba.shape[:2];bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update();objects=[o for o in bpy.context.scene.objects if o.type=='MESH'];tree,owners,_=_tree(objects);triangles=[];colors={};topology=[]
 for obj in objects:
  obj.data.calc_loop_triangles();triangles.extend((obj,t)for t in obj.data.loop_triangles);bm=bmesh.new();bm.from_mesh(obj.data);topology.append(dict(object=obj.name,nonmanifold_edges=sum(not e.is_manifold for e in bm.edges),volume=bm.calc_volume(signed=True),minimum_z=min((obj.matrix_world@v.co).z for v in obj.data.vertices),maximum_z=max((obj.matrix_world@v.co).z for v in obj.data.vertices)));bm.free()
  for i,m in enumerate(obj.data.materials):
   tex=next(n for n in m.node_tree.nodes if n.type=='TEX_IMAGE');colors[(obj.name,i)]=np.array(Image.open(io.BytesIO(bytes(tex.image.packed_file.data))).convert('RGBA'));shader=next(n for n in m.node_tree.nodes if n.type=='BSDF_PRINCIPLED');assert not shader.inputs['Alpha'].links and shader.inputs['Alpha'].default_value==1
 missing=[];foreign=[];wrong=[];maximum=0.;tested=0
 for y in range(h):
  for x in range(w):
   p,_,ti,_=tree.ray_cast(Vector((x+.5-w/2,-(y+.5-h/2)/SIN,0))+RAY*500,-RAY)
   if rgba[y,x,3]==0:
    if p is not None:foreign.append([x,y])
    continue
   if p is None:missing.append([x,y]);continue
   obj,t=triangles[ti];q=barycentric_transform(p,*[obj.matrix_world@obj.data.vertices[i].co for i in t.vertices],*[Vector((*obj.data.uv_layers.active.data[i].uv,0))for i in t.loops]);sx=q.x*w;sy=(1-q.y)*h;maximum=max(maximum,abs(sx-x-.5),abs(sy-y-.5));actual=colors[(obj.name,t.material_index)][int(sy),int(sx),:3]
   if t.material_index!=0 or not np.array_equal(actual,rgba[y,x,:3]):wrong.append(dict(pixel=[x,y],material=t.material_index,rgb=actual.tolist(),expected=rgba[y,x,:3].tolist()))
   tested+=1
 valid=not(missing or foreign or wrong)and tested==1011 and maximum<.01 and all(t['nonmanifold_edges']==0 and t['volume']>0 for t in topology)
 (worker/'saved-native-guard.json').write_text(json.dumps(dict(status='PASS'if valid else'HOLD',model_sha256=sha(model),source_sha256=s['source_sha256'],opaque_centers=tested,missing=missing,foreign=foreign,wrong_color=wrong,maximum_texel_error=maximum,topology=topology,actual_coverage_is_geometry=True,scope='No alpha discard. Reopened actual closed meshes and native first-hit RGB. Flat candidate only; contact and all-placement tests are separate.'),indent=2)+'\n');print(json.dumps(dict(status='PASS'if valid else'HOLD',missing=len(missing),foreign=len(foreign),wrong=len(wrong),nonmanifold=sum(t['nonmanifold_edges']for t in topology))))
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
