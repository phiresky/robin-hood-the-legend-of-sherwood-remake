"""Verify reopened clump native first-hit texels and exact packed source bytes."""
from pathlib import Path
import sys,json,hashlib
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.geometry import barycentric_transform
P=Path(__file__).resolve().parent;sys.path.insert(0,str(P))
from restart6_source_gap_audit import OUT,RAY,SIN
from refinement_review import _tree
from render_slots import acquire,release
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 worker=OUT/'restart11-hiding-mound/clump-volumes-v2';r=json.loads((worker/'validation.json').read_text());src=json.loads((OUT/'restart9-hiding-scatter/mound-flat-v2/validation.json').read_text());rgba=np.array(Image.open(src['source']).convert('RGBA'));h,w=rgba.shape[:2];model=worker/'model.blend';assert sha(model)==r['model_sha256'];bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update();objects=[o for o in bpy.context.scene.objects if o.type=='MESH'];tree,owners,_=_tree(objects);triangles=[]
 for o in objects:
  o.data.calc_loop_triangles();triangles.extend((o,t)for t in o.data.loop_triangles)
  for m in o.data.materials:
   image=next(n.image for n in m.node_tree.nodes if n.type=='TEX_IMAGE');assert hashlib.sha256(bytes(image.packed_file.data)).hexdigest()==src['source_sha256']
 missing=[];foreign=[];wrong=[];maximum=0.;tested=0
 for y in range(h):
  for x in range(w):
   p,n,ti,d=tree.ray_cast(Vector((x+.5-w/2,-(y+.5-h/2)/SIN,0))+RAY*500,-RAY)
   if rgba[y,x,3]==0:
    if p is not None:foreign.append([x,y])
    continue
   if p is None:missing.append([x,y]);continue
   o,t=triangles[ti];mesh=o.data
   if t.material_index!=0:wrong.append([x,y,t.material_index]);continue
   q=barycentric_transform(p,*[o.matrix_world@mesh.vertices[i].co for i in t.vertices],*[Vector((*mesh.uv_layers.active.data[i].uv,0))for i in t.loops]);sx=q.x*w;sy=(1-q.y)*h;maximum=max(maximum,abs(sx-x-.5),abs(sy-y-.5));assert int(sx)==x and int(sy)==y;tested+=1
 assert not missing and not foreign and not wrong and tested==1011 and maximum<.01
 (worker/'saved-native-guard.json').write_text(json.dumps(dict(status='PASS',model_sha256=sha(model),source_sha256=src['source_sha256'],opaque_source_centers=tested,foreign_transparent_centers=foreign,missing=missing,wrong_material=wrong,maximum_texel_center_error=maximum,scope='Reopened eight closed clumps. Explicit physical alpha coverage, exact observed first-hit materials/UV/source texels. No support variants or rear texture completion.'),indent=2)+'\n')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
