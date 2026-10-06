"""Preserve upper surface normals and exact observed grazing lower colors."""
import sys,json
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
from restart6_source_gap_audit import ROOT,OUT,RAY,SIN
from refinement_review import _tree
from review_sunlight import _surface,_interpolated_normal
from evidence_io import sha,write_json
from render_slots import acquire,release

def main(number):
 parent=ROOT/f'tree{number}-toe-union-fit-v{9 if number==19 else 11}'/'model.blend';out=ROOT/f'tree{number}-toe-finished-v12';out.mkdir(exist_ok=False);prior=json.load(open(ROOT/f'baseline-audit-{number}-v3/report.json'));source=Path(prior['source']);asset=f'croisement02-tree-{number}';limit=43 if number==19 else 71
 def wood():return[o for o in bpy.context.scene.objects if o.type=='MESH'and o.get('asset_group')==asset and o.get('projection_component')!='crown']
 bpy.ops.wm.open_mainfile(filepath=str(source));bpy.context.view_layer.update();_,vertices,triangles,normals=_surface(wood());surface=BVHTree.FromPolygons(vertices,triangles,all_triangles=True);bpy.ops.wm.open_mainfile(filepath=str(parent));bpy.context.view_layer.update();objects=wood();normalcount=0
 for o in objects:
  transform=o.matrix_world.to_3x3().inverted().transposed();back=transform.inverted();values=[]
  for v in o.data.vertices:
   p=o.matrix_world@v.co;n=(transform@v.normal).normalized();t=max(0,min(1,(p.z-(limit-9))/9));t=t*t*(3-2*t)
   if t:
    q,_,i,d=surface.find_nearest(p);native=_interpolated_normal(q,triangles[i],vertices,normals[i]);n=n.lerp(native,t).normalized();normalcount+=1
   values.append(tuple((back@n).normalized()))
  o.data.normals_split_custom_set_from_vertices(values)
 # Preserve explicit native samples even where a newly fitted face grazes the source ray.
 item=next(x for x in json.load(open(OUT/'review-mask-inventory.json'))['masks']if x['index']==number);ox,oy=item['box_top_left'];w,h=item['box_size'];yy,xx=np.where(np.array(Image.open(item['png']))>0);tree,_,_=_tree(objects);rows=[]
 for o in objects:o.data.calc_loop_triangles();rows.extend((o,t)for t in o.data.loop_triangles)
 fixes=[]
 for y,x in zip(yy+oy,xx+ox):
  p,n,i,d=tree.ray_cast(Vector((x+.5,-(y+.5)/SIN,0))+RAY*6000,-RAY)
  if p is None or p.z>limit:continue
  o,t=rows[i];mat=o.data.materials[t.material_index]
  if not any(n.type=='TEX_IMAGE'and n.image and n.image.name.startswith(f'native{number}')for n in mat.node_tree.nodes):fixes.append(dict(pixel=[int(x),int(y)],object=o.name,face=t.polygon_index))
 if fixes:
  a=np.array(Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA'))[oy:oy+h,ox:ox+w].copy();a[:,:,3]=0
  for r in fixes:x,y=r['pixel'];a[y-oy,x-ox,3]=255
  Image.fromarray(a).save(out/f'native{number}-grazing.png');image=bpy.data.images.load(str(out/f'native{number}-grazing.png'));image.pack();cache={};assigned=set()
  for r in fixes:
   keyface=(r['object'],r['face'])
   if keyface in assigned:continue
   assigned.add(keyface);o=bpy.data.objects[r['object']];face=o.data.polygons[r['face']];slot=face.material_index;key=(o.name,slot)
   if key not in cache:
    mat=o.data.materials[slot].copy();mat.name+=' / observed grazing continuation';nodes=mat.node_tree.nodes;socket=next(n for n in nodes if n.type=='OUTPUT_MATERIAL').inputs['Surface'];old=socket.links[0].from_socket;tex=nodes.new('ShaderNodeTexImage');tex.image=image;tex.interpolation='Closest';tex.extension='CLIP';coord=nodes.new('ShaderNodeUVMap');coord.uv_map='Continuous toe native projection';mix=nodes.new('ShaderNodeMixRGB');mat.node_tree.links.new(coord.outputs['UV'],tex.inputs['Vector']);mat.node_tree.links.new(tex.outputs['Alpha'],mix.inputs[0]);mat.node_tree.links.new(old,mix.inputs[1]);mat.node_tree.links.new(tex.outputs['Color'],mix.inputs[2]);mat.node_tree.links.new(mix.outputs[0],socket);o.data.materials.append(mat);cache[key]=len(o.data.materials)-1
   face.material_index=cache[key]
 bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'),compress=True);write_json(out/'finish.json',dict(parent_sha256=sha(parent),source_sha256=sha(source),model_sha256=sha(out/'model.blend'),normal_vertices=normalcount,grazing_fixes=fixes,scope='Geometry/UV unchanged; upper original surface normals restored with nine-unit lower transition. Explicit grazing source pixels restored, no synthesized appearance.'))
if __name__=='__main__':
 acquire()
 try:main(int(sys.argv[sys.argv.index('--')+1]))
 finally:release()
