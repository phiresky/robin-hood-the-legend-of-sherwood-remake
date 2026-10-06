"""Check retained geometry/UV records and native texels after local fork reconstruction."""
import sys,json,hashlib
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from mathutils.geometry import barycentric_transform
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
from restart6_tree24_contour import ROOT,OUT,RAY,SIN
from evidence_io import sha,write_json
from refinement_review import _tree
from bake_texture_candidate import pixels
from render_slots import acquire,release

def records(path):
 bpy.ops.wm.open_mainfile(filepath=str(path));bpy.context.view_layer.update();result=[]
 for o in bpy.context.scene.objects:
  if o.type!='MESH'or'Crown'in o.name:continue
  for face in o.data.polygons:
   mat=o.data.materials[face.material_index]
   for li in face.loop_indices:
    p=o.matrix_world@o.data.vertices[o.data.loops[li].vertex_index].co
    if 98-.001<=p.z<=152+.001:continue
    key=(mat.name,tuple((u.name,tuple(round(x,6)for x in u.data[li].uv))for u in o.data.uv_layers if u.name!='Exact lower contour native projection'));result.append((key,p.copy()))
 return result
acquire()
try:
 model=ROOT/'tree24-fork-union-v10/model.blend';source=ROOT/'tree24-contour-v2/model.blend';old=records(source);new=records(model);lookup={}
 for key,p in new:lookup.setdefault(key,[]).append(p)
 distances=[];missing=[]
 for key,p in old:
  if key not in lookup:missing.append(dict(position=list(p),reason='Missing original material/UV tuple'));continue
  d=min((p-q).length for q in lookup[key]);distances.append(d)
  if d>.0004:missing.append(dict(position=list(p),reason='Retained geometry drift',distance=d))
 wood=[o for o in bpy.context.scene.objects if o.type=='MESH'and'Crown'not in o.name];tree,owners,_=_tree(wood);tris=[]
 for o in wood:o.data.calc_loop_triangles();tris.extend((o,t)for t in o.data.loop_triangles)
 yy,xx=np.where(np.array(Image.open(ROOT/'source-audit-v1/exposed-24.png'))>0);native=np.array(Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA'));samples=[];cache={}
 for y,x in zip(yy,xx):
  p,n,i,d=tree.ray_cast(Vector((x+.5,-(y+.5)/SIN,0))+RAY*6000,-RAY);assert p is not None;o,t=tris[i];mat=o.data.materials[t.material_index];node=next((n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE'and n.image and n.image.name.startswith('native24')),None)
  if node is None:samples.append(dict(pixel=[int(x),int(y)],exact=False,reason='No native overlay at saved first hit'));continue
  uv=o.data.uv_layers['Exact lower contour native projection'];q=barycentric_transform(p,*[o.matrix_world@o.data.vertices[j].co for j in t.vertices],*[Vector((*uv.data[j].uv,0))for j in t.loops]);a=cache.get(node.image.name)
  if a is None:a=np.rint(pixels(node.image)*255).astype('uint8');cache[node.image.name]=a
  h,w=a.shape[:2];value=a[min(h-1,max(0,int(q.y*h))),min(w-1,max(0,int(q.x*w)))];samples.append(dict(pixel=[int(x),int(y)],exact=bool(np.array_equal(value,native[y,x])),actual=value.tolist(),expected=native[y,x].tolist()))
 result=dict(model_sha256=sha(model),parent_sha256=sha(source),retained_outside_joint_records=len(old),retained_original_uv_material_tuples_present=not missing,max_world_float_difference=max(distances),missing=missing,native_edge_samples=samples,scope='Original world/UV/material loop records outsideZ98..152 retained with explicit floating transform tolerance0.0004; additional boundary interpolation separately inferred. Exact four native edge RGBA samples.');write_json(model.parent/'retained-surface-native-guard.json',result);print(result,flush=True)
finally:release()
