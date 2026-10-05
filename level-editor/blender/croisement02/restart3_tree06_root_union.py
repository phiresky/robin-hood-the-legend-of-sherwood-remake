"""Join source-supported root tips into one rounded volume below the bank."""
import sys,json,math
from pathlib import Path
import bpy,bmesh,numpy as np
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from tree_geometry import SIN,COS
from render_slots import acquire,release
from evidence_io import sha,write_json
from restart3_tree06_root_correction import fingerprint


def main():
 base=OUT/'restart3-tree06-root';source=base/'collar-v3/model.blend';dest=base/'collar-v6';dest.mkdir(exist_ok=False)
 assert sha(source)=='a9afd2b584cf283c82768a4b12f16d4f44af3e2d530da8882714b180fbfdea90'
 bpy.ops.wm.open_mainfile(filepath=str(source));bpy.context.view_layer.update()
 obj=bpy.data.objects['Northwest Tree 06 / Root collar continuation']
 originals=[o for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')=='croisement02-tree-06' and o!=obj]
 fixed={o.name:fingerprint(o)for o in originals}
 # Only the inferred underside changes before union. The measured front is
 # retained and the new connection stays below the verified Z43.949 bank.
 for v in obj.data.vertices:
  if v.co.z<43.:
   delta=32.-v.co.z
   v.co.y-=COS/SIN*delta
   v.co.z=32.
 obj.data.update()
 bpy.ops.object.select_all(action='DESELECT');obj.select_set(True);bpy.context.view_layer.objects.active=obj
 bevel=obj.modifiers.new('Subpixel rounded root edges','BEVEL');bevel.width=.18;bevel.segments=3;bevel.limit_method='ANGLE';bevel.angle_limit=math.radians(60)
 bpy.ops.object.modifier_apply(modifier=bevel.name)
 bpy.ops.mesh.primitive_uv_sphere_add(segments=64,ring_count=32,radius=1,location=(650,-980,32))
 base_obj=bpy.context.object;base_obj.name='Private buried root connection';base_obj.scale=(60,45,10.5)
 base_obj.data.materials.append(obj.data.materials[1]);bpy.ops.object.transform_apply(location=False,rotation=False,scale=True)
 bpy.ops.object.select_all(action='DESELECT');obj.select_set(True);bpy.context.view_layer.objects.active=obj
 modifier=obj.modifiers.new('Buried connected root volume','BOOLEAN');modifier.operation='UNION';modifier.solver='EXACT';modifier.object=base_obj
 bpy.ops.object.modifier_apply(modifier=modifier.name);bpy.data.objects.remove(base_obj,do_unlink=True)
 # Re-evaluate native projection after bevel interpolation, rather than
 # relying on UV interpolation to represent moved vertices exactly.
 uv=obj.data.uv_layers['Root native projection']
 for face in obj.data.polygons:
  face.use_smooth=face.material_index!=0
  if face.material_index==0:
   for loop in face.loop_indices:
    p=obj.matrix_world@obj.data.vertices[obj.data.loops[loop].vertex_index].co
    uv.data[loop].uv=(p.x/1792,1-(-SIN*p.y-COS*p.z)/1152)
 bm=bmesh.new();bm.from_mesh(obj.data);nonmanifold=sum(not e.is_manifold for e in bm.edges)
 remaining=set(bm.verts);sizes=[]
 while remaining:
  stack=[remaining.pop()];size=0
  while stack:
   v=stack.pop();size+=1
   for e in v.link_edges:
    other=e.other_vert(v)
    if other in remaining:remaining.remove(other);stack.append(other)
  sizes.append(size)
 bm.free()
 write_json(dest/'topology-diagnostic.json',dict(nonmanifold_edges=nonmanifold,components=sizes))
 assert nonmanifold==0,nonmanifold
 assert len(sizes)==1,sizes
 assert all(fingerprint(o)==fixed[o.name]for o in originals)
 bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(dest/'model.blend'))
 prior=json.loads((base/'collar-v3/report.json').read_text())
 write_json(dest/'report.json',dict(status='Private coherent root volume; saved-material review pending',input_model=prior['input_model'],input_sha256=prior['input_sha256'],parent_model_sha256=sha(source),model_sha256=sha(dest/'model.blend'),unchanged_objects=list(fixed),unchanged_existing_geometry_uv_material_transform=True,nonmanifold_edges=nonmanifold,connected_components=sizes,buried_connector=dict(center=[650,-980,32],radii=[60,45,10.5],maximum_z=42.5,verified_bank_z_min=43.94873046875),bevel_width=.18,source_uvs_reprojected_after_bevel=True,limitations=['Buried root connection and unknown underside are inferred.','Native313 and neighboring bank131 must pass fresh saved-model render verification.','New geometry is not user approved.']))
 print(dest,flush=True)


if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
