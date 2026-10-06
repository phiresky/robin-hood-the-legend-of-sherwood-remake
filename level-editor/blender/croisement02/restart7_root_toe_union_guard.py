"""Read-only reopened upper, crown, topology and native lower appearance guards."""
import sys,json,hashlib
from pathlib import Path
import bpy,bmesh,numpy as np
from mathutils import Vector
from mathutils.geometry import barycentric_transform
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
from restart6_source_gap_audit import ROOT,OUT,RAY,SIN
from refinement_workspace import _geometry
from refinement_review import _tree
from restart6_tree38_contour import covered
from evidence_io import sha,write_json
from render_slots import acquire,release
from bake_texture_candidate import pixels

def main(number):
 model=ROOT/f'tree{number}-toe-union-fit-v9'/'model.blend';out=model.parent;prior=json.load(open(ROOT/f'baseline-audit-{number}-v3/report.json'));source=Path(prior['source']);asset=f'croisement02-tree-{number}';limit=43 if number==19 else 71
 def objects():return[o for o in bpy.context.scene.objects if o.type=='MESH'and o.get('asset_group')==asset]
 def upper(objects):
  result=[]
  for o in objects:
   if o.get('projection_component')=='crown':continue
   o.data.calc_loop_triangles()
   for t in o.data.loop_triangles:
    p=[o.matrix_world@o.data.vertices[i].co for i in t.vertices]
    if min(v.z for v in p)<=limit+.01:continue
    result.append(tuple(sorted(tuple(round(v,5)for v in p0)for p0 in p)))
  return sorted(result)
 bpy.ops.wm.open_mainfile(filepath=str(source));bpy.context.view_layer.update();old=objects();upper_old=upper(old);crowns={o.name:_geometry(o,protect_appearance=True)for o in old if o.get('projection_component')=='crown'};packed={im.name:hashlib.sha256(bytes(im.packed_file.data)).hexdigest()for o in old for mat in o.data.materials if mat and mat.use_nodes for node in mat.node_tree.nodes if node.type=='TEX_IMAGE'and(im:=node.image)and im.packed_file};bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update();own=objects();wood=[o for o in own if o.get('projection_component')!='crown'];upper_new=upper(own);crown_ok=all(_geometry(o,protect_appearance=True)==crowns[o.name]for o in own if o.get('projection_component')=='crown');retained={im.name:hashlib.sha256(bytes(im.packed_file.data)).hexdigest()for im in bpy.data.images if im.packed_file};images_ok=all(retained.get(k)==v for k,v in packed.items());verts=[];faces=[];tris=[]
 for o in wood:
  offset=len(verts);verts.extend(o.matrix_world@v.co for v in o.data.vertices);faces.extend(tuple(offset+i for i in f.vertices)for f in o.data.polygons);o.data.calc_loop_triangles();tris.extend((o,t)for t in o.data.loop_triangles)
 mesh=bpy.data.meshes.new('Read only assembled audit');mesh.from_pydata(verts,[],faces);bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.remove_doubles(bm,verts=list(bm.verts),dist=.00002);topology=dict(nonmanifold=sum(not e.is_manifold for e in bm.edges),degenerate=sum(f.calc_area()<1e-9 for f in bm.faces));bm.free();bpy.data.meshes.remove(mesh);tree,_,_=_tree(wood);item=next(x for x in json.load(open(OUT/'review-mask-inventory.json'))['masks']if x['index']==number)
 from PIL import Image
 mask=np.array(Image.open(item['png']))>0;yy,xx=np.where(mask);ox,oy=item['box_top_left'];native=np.array(Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA'));exact=0;fail=[];cache={};lower_hits=0
 for y,x in zip(yy+oy,xx+ox):
  hit,n,index,d=tree.ray_cast(Vector((x+.5,-(y+.5)/SIN,0))+RAY*6000,-RAY)
  if hit is None or hit.z>limit:continue
  lower_hits+=1;o,t=tris[index];mat=o.data.materials[t.material_index];node=next((n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE'and n.image and n.image.name.startswith(f'native{number}')),None)
  if node is None:fail.append(dict(pixel=[int(x),int(y)],reason='first lower surface has no native overlay',object=o.name,face=t.polygon_index));continue
  uv=o.data.uv_layers['Continuous toe native projection'];q=barycentric_transform(hit,*[o.matrix_world@o.data.vertices[j].co for j in t.vertices],*[Vector((*uv.data[j].uv,0))for j in t.loops]);image=node.image
  if image.name not in cache:cache[image.name]=np.rint(pixels(image)*255).astype('uint8')
  a=cache[image.name];h,w=a.shape[:2];value=a[min(h-1,max(0,int(q.y*h))),min(w-1,max(0,int(q.x*w)))];ok=np.array_equal(value,native[y,x]);exact+=int(ok)
  if not ok:fail.append(dict(pixel=[int(x),int(y)],reason='native RGBA differs',value=value.tolist(),expected=native[y,x].tolist()))
 write_json(out/'saved-guard-v2.json',dict(model_sha256=sha(model),source_sha256=sha(source),upper_triangles_exact=upper_old==upper_new,upper_triangles_old=len(upper_old),upper_triangles_new=len(upper_new),crown_exact=crown_ok,retained_packed_images_exact=images_ok,missing_or_changed_packed_images=[k for k,v in packed.items()if retained.get(k)!=v],assembled_topology=topology,minimum_z=min(v.z for v in verts),lower_native_hits=lower_hits,lower_native_exact=exact,lower_native_failures=fail,scope='Read-only explicit native overlay samples; whole upper materials remain from approved parent. No synthesized appearance approval.'))
if __name__=='__main__':
 acquire()
 try:main(int(sys.argv[sys.argv.index('--')+1]))
 finally:release()
