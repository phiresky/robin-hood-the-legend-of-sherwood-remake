"""Retain exact approved wagon source shaders above generated unknown fallback."""
import sys,json,hashlib
from pathlib import Path
from array import array
import bpy
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender'),str(Path(__file__).parent)]
from render_slots import acquire,release
from bake_texture_candidate import snapshot,pixels,array_hash
from render_multiview_asset import render
from refinement_review import _tile
from catalog import OUT
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
E=OUT/'restart4-south-cart-texture/approved-fill-v1/experiment';D=E/'native-retained-v1';meta=json.loads((E/'views.json').read_text());names=set(meta['object_names'])
def images(mat):
 return {n.name:array_hash(pixels(n.image)) for n in mat.node_tree.nodes if n.type=='TEX_IMAGE' and n.image}
acquire()
try:
 assert not D.exists()
 bpy.ops.wm.open_mainfile(filepath=str(E/'approved-model.blend'));scene=bpy.data.scenes[meta['scene_name']];before=snapshot(scene,names)
 original={name:dict(slots=[p.material_index for p in scene.objects[name].data.polygons],images={i:images(m) for i,m in enumerate(scene.objects[name].data.materials) if m and m.use_nodes and images(m)}) for name in names}
 bpy.ops.wm.open_mainfile(filepath=str(E/'bake-v1/worker.blend'));scene=bpy.data.scenes[meta['scene_name']];bpy.context.window.scene=scene;assert snapshot(scene,names)==before;records=[]
 for name in sorted(names):
  obj=scene.objects[name];cache={}
  for face,oldslot in zip(obj.data.polygons,original[name]['slots']):
   if oldslot not in original[name]['images']:continue
   native=obj.data.materials[oldslot];assert images(native)==original[name]['images'][oldslot]
   generated=obj.data.materials[face.material_index]
   if not generated.get('source_ownership_bake'):assert face.material_index==oldslot;continue
   key=(oldslot,face.material_index)
   if key not in cache:
    restored=native.copy();restored.name=native.name+' / exact native and inferred fill';mix=next(n for n in restored.node_tree.nodes if n.type=='MIX_RGB');assert not mix.inputs[1].is_linked
    gn=next(n for n in generated.node_tree.nodes if n.type=='TEX_IMAGE');gnuv=gn.inputs['Vector'].links[0].from_node.uv_map
    fallback=restored.node_tree.nodes.new('ShaderNodeTexImage');fallback.image=gn.image;fallback.interpolation='Linear';fallback.extension='EXTEND';uv=restored.node_tree.nodes.new('ShaderNodeUVMap');uv.uv_map=gnuv;restored.node_tree.links.new(uv.outputs['UV'],fallback.inputs['Vector']);restored.node_tree.links.new(fallback.outputs['Color'],mix.inputs[1]);obj.data.materials.append(restored);cache[key]=len(obj.data.materials)-1
   face.material_index=cache[key]
  records.append(dict(object=name,restored_material_pairs=len(cache),native_images=original[name]['images']))
 assert snapshot(scene,names)==before
 D.mkdir();bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(D/'worker.blend'),compress=True);digest=sha(D/'worker.blend');bpy.ops.wm.open_mainfile(filepath=str(D/'worker.blend'));scene=bpy.data.scenes[meta['scene_name']];bpy.context.window.scene=scene;assert snapshot(scene,names)==before
 for name in names:
  for slot,expected in original[name]['images'].items():assert images(scene.objects[name].data.materials[slot])==expected
 scene.render.engine='CYCLES';scene.cycles.samples=8;render(E/'views.json',D/'actual',width=384);buffers=[]
 for i in range(8):
  im=bpy.data.images.load(str(D/'actual'/f'view-{i}-textured.png'),check_existing=False);a=array('f',[0])*len(im.pixels);im.pixels.foreach_get(a);buffers.append(a);bpy.data.images.remove(im)
 _tile(buffers,384,384,D/'actual/textured.png')
 (D/'native-preservation.json').write_text(json.dumps(dict(status='PASS',model_sha256=digest,approved_preparation_sha256=sha(E/'approved-model.blend'),baked_parent_sha256=sha(E/'bake-v1/worker.blend'),parent_guard_sha256=sha(E/'bake-v1/reopened-preservation.json'),geometry_unchanged=True,reopened_geometry_pass=True,native_rgba_images_exact=True,original_native_uv_maps_retained=True,source_shader_alpha_selects_native_over_generated_fallback=True,objects=records,actual_review='pending'),indent=2)+'\n')
finally:release()
