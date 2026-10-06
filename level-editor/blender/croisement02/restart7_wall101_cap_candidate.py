"""Bounded masonry cap contour correction with native-front reprojection and frozen neighbors."""
import sys,json,math
from pathlib import Path
import bpy,numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from tree_geometry import SIN,COS,RAY
from evidence_io import sha,write_json
from render_slots import acquire,release
from restart3_tree06_root_correction import fingerprint
from restart3_tree06_root_review import configure
from restart2_sign_neighbors import camera_to,render
BASE=OUT/'texture-fill-round-1/croisement02-east-stone-wall-and-gate/experiment/bake-v1/worker.blend';D=OUT/'restart7-fence-residual/wall101-cap-v2';ASSET='croisement02-east-stone-wall-and-gate'
def main():
 D.mkdir(exist_ok=False);assert sha(BASE)=='be2025b12af93d5e6bfe1c0e14dd7e6ffd384d16842a564624bfd91690952ff2'
 bpy.ops.wm.open_mainfile(filepath=str(BASE));bpy.context.view_layer.update();scene=bpy.context.scene;objects=[o for o in scene.objects if o.type=='MESH'and o.get('asset_group')==ASSET];obj=next(o for o in objects if o.get('source_node')=='building-010');others={o.name:fingerprint(o)for o in objects if o!=obj};old=[obj.matrix_world@v.co for v in obj.data.vertices];inverse=obj.matrix_world.inverted();changed=[]
 for i,p in enumerate(old):
  sy=-SIN*p.y-COS*p.z
  if 1538<p.x<1548 and 844.7<sy<848 and p.z>45:
   wx=min(1,max(0,(p.x-1538)/3),max(0,(1548-p.x)/2));dz=max(0,(sy-843.8)/COS)*wx
   if dz>.0001:obj.data.vertices[i].co=inverse@(p+Vector((0,0,dz)));changed.append(i)
 obj.data.update();uv=obj.data.uv_layers.new(name='Native cap projection')
 for loop in obj.data.loops:
  p=obj.matrix_world@obj.data.vertices[loop.vertex_index].co;uv.data[loop.index].uv=(p.x/1792,1-(-SIN*p.y-COS*p.z)/1152)
 source=bpy.data.images.load(str(OUT/'animation-references/composite-frame-0.png'));source.pack();mask=np.zeros((1152,1792),np.uint8);native=np.asarray(Image.open(OUT/'baseline/masks/000101.png').convert('L'));mask[829:829+native.shape[0],1419:1419+native.shape[1]]=native;Image.fromarray(mask).save(D/'native-domain.png');own=bpy.data.images.load(str(D/'native-domain.png'));own.colorspace_settings.name='Non-Color';own.pack()
 def material(front):
  m=bpy.data.materials.new('Cap observed native front'if front else 'Cap inferred gray');m.use_nodes=True;n=m.node_tree.nodes;n.clear();out=n.new('ShaderNodeOutputMaterial');em=n.new('ShaderNodeEmission');em.inputs['Color'].default_value=(.22,.22,.22,1);m.node_tree.links.new(em.outputs[0],out.inputs['Surface'])
  if front:
   u=n.new('ShaderNodeUVMap');u.uv_map=uv.name;mix=n.new('ShaderNodeMixRGB');mix.inputs[1].default_value=(.22,.22,.22,1)
   for image,slot in [(source,2),(own,0)]:
    tex=n.new('ShaderNodeTexImage');tex.image=image;tex.interpolation='Closest';tex.extension='CLIP';m.node_tree.links.new(u.outputs[0],tex.inputs['Vector']);m.node_tree.links.new(tex.outputs['Color'],mix.inputs[slot])
   m.node_tree.links.new(mix.outputs[0],em.inputs['Color'])
  return m
 offset=len(obj.data.materials);obj.data.materials.append(material(False));obj.data.materials.append(material(True));changedset=set(changed);faces=[]
 for poly in obj.data.polygons:
  if any(i in changedset for i in poly.vertices):poly.material_index=offset+int((obj.matrix_world.to_3x3()@poly.normal).normalized().dot(RAY)>=.05);faces.append(poly.index)
 assert others=={o.name:fingerprint(o)for o in objects if o!=obj};bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(D/'model.blend'),compress=True)
 write_json(D/'changes.json',dict(base=str(BASE),base_sha256=sha(BASE),model_sha256=sha(D/'model.blend'),asset_id=ASSET,receiver=obj.name,source_node='building-010',vertices=changed,faces=faces,maximum_world_displacement=max((obj.matrix_world@obj.data.vertices[i].co-old[i]).length for i in changed),all_other_receivers_exact=True,unchanged_vertices_exact=all((obj.matrix_world@v.co-old[i]).length<.001 for i,v in enumerate(obj.data.vertices)if i not in changedset),geometry_scope='Smooth local upward cap contour adjustment; no detached pixel solids, ground or fence changes.'))
 bpy.ops.wm.open_mainfile(filepath=str(D/'model.blend'));bpy.context.view_layer.update();scene=bpy.context.scene;objects=[o for o in scene.objects if o.type=='MESH'and o.get('asset_group')==ASSET]
 for o in scene.objects:
  if o.type=='MESH':o.hide_render=o not in objects
 camera=configure(scene);scene.cycles.samples=8;center=Vector((1543.5,(-845-COS*50)/SIN,50));camera.data.ortho_scale=100
 world=bpy.data.worlds.new('Cap solid review world');world.use_nodes=True;world.node_tree.nodes['Background'].inputs['Strength'].default_value=.8;scene.world=world;light=bpy.data.lights.new('Cap solid sun','SUN');light.energy=2;sun=bpy.data.objects.new(light.name,light);scene.collection.objects.link(sun);sun.rotation_euler=(.5,-.6,-.4)
 solid=bpy.data.materials.new('Cap solid neutral');solid.use_nodes=True;solid.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value=(.45,.45,.45,1)
 for mode in ['actual','solid']:
  scene.view_layers[0].material_override=solid if mode=='solid'else None;sheet=Image.new('RGB',(1536,816),(45,45,45))
  for i in range(8):
   angle=i*math.pi/4;camera_to(camera,center,Vector((math.sin(angle)*COS,-math.cos(angle)*COS,SIN)));pic=render(scene,D/f'{mode}-{i}.png');sheet.paste(pic,(i%4*384,i//4*408),pic.getchannel('A'));ImageDraw.Draw(sheet).text((i%4*384+5,i//4*408+388),f'{mode} wall contact {i}; native first',fill='white')
  sheet.save(D/f'{mode}-eight.png')
 # Exact native crop framing for before/after source comparison.
 shots=[];box=(1510,815,1574,879)
 for label,path in [('before',BASE),('after',D/'model.blend')]:
  bpy.ops.wm.open_mainfile(filepath=str(path));bpy.context.view_layer.update();scene=bpy.context.scene
  for o in scene.objects:
   if o.type=='MESH':o.hide_render=o.get('asset_group')!=ASSET
  camera=configure(scene);camera.data.ortho_scale=64;camera_to(camera,Vector((1542,-847/SIN,0)),RAY);shots.append(render(scene,D/f'native-{label}.png'))
 sheet=Image.new('RGB',(1152,408),(45,45,45));src=Image.open(OUT/'animation-references/composite-frame-0.png').crop(box).resize((384,384),Image.Resampling.NEAREST).convert('RGBA')
 for i,pic in enumerate([src,*shots]):sheet.paste(pic,(i*384,0),pic.getchannel('A'));ImageDraw.Draw(sheet).text((i*384+5,388),['Original source','Approved baseline','Private cap candidate'][i],fill='white')
 sheet.save(D/'source-comparison.png');print(D,flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
