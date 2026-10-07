"""Show retained painted terrain beneath approved physical crowns, without edits."""
import hashlib,io,json,math,struct,sys
from pathlib import Path
import bpy
from mathutils import Vector
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from render_views import render_views
B=ROOT/'level-editor/work/croisement03-refinement/restart2';OUT=B/'trio-tree-integration-v1/terrain-contact-before-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 OUT.mkdir(exist_ok=False);acquire()
 try:
  specs=[(12,1),(13,2),(14,1)];paths=[B/f'tree{n}-approved-wood-texture-v{r}/source-restored-fill-v1/worker.blend' for n,r in specs];guards={str(p):sha(p) for p in paths};bpy.ops.wm.open_mainfile(filepath=str(paths[0]));scene=bpy.data.scenes['Tree13 isolated wood'];bpy.context.window.scene=scene
  for (tree,_),path in zip(specs[1:],paths[1:]):
   with bpy.data.libraries.load(str(path),link=False) as (a,b):b.objects=list(a.objects)
   allowed={f'croisement03-tree-{tree}',f'croisement03-arbre06-fragment-tree{tree}-provisional','croisement03-local75-provisional' if tree==13 else ''}
   for o in b.objects:
    if o.type=='MESH' and o.get('asset_group') in allowed:scene.collection.objects.link(o);o.hide_render=False
  trees=[o for o in scene.objects if o.type=='MESH'];terrain=ROOT/'level-editor/library/3d-assets/croisement03/croisement03-terrain/model.glb';guards[str(terrain)]=sha(terrain);data=terrain.read_bytes();n=struct.unpack_from('<I',data,12)[0];g=json.loads(data[20:20+n]);v=g['bufferViews'][g['images'][0]['bufferView']];imagepath=OUT/'ground-original.jpg';imagepath.write_bytes(data[28+n+v.get('byteOffset',0):28+n+v.get('byteOffset',0)+v['byteLength']]);im=bpy.data.images.load(str(imagepath));material=bpy.data.materials.new('Unchanged current painted ground');material.use_nodes=True;nodes=material.node_tree.nodes;nodes.clear();tex=nodes.new('ShaderNodeTexImage');tex.image=im;tex.interpolation='Linear';em=nodes.new('ShaderNodeEmission');out=nodes.new('ShaderNodeOutputMaterial');material.node_tree.links.new(tex.outputs['Color'],em.inputs[0]);material.node_tree.links.new(em.outputs[0],out.inputs[0]);mesh=bpy.data.meshes.new('Current ground quad');mesh.from_pydata([(0,0,0),(1408,0,0),(1408,-960/math.sin(math.radians(35)),0),(0,-960/math.sin(math.radians(35)),0)],[],[(0,1,2,3)]);mesh.materials.append(material);uv=mesh.uv_layers.new(name='UVMap')
  for li,val in zip(mesh.polygons[0].loop_indices,((0,1),(1,1),(1,0),(0,0))):uv.data[li].uv=val
  ground=bpy.data.objects.new('Unchanged ground full quad',mesh);scene.collection.objects.link(ground);views={}
  for i in (0,4):
   cam=bpy.data.objects[f'Tree13 view{i}'];a=i*math.tau/8;direction=Vector((math.sin(a)*math.cos(math.radians(35)),-math.cos(a)*math.cos(math.radians(35)),math.sin(math.radians(35))));cam.location=Vector((1060,-270,165))+direction*1400;cam.rotation_euler=(-direction).to_track_quat('-Z','Y').to_euler();cam.data.ortho_scale=500;views[f'view-{i}']=cam.name
  scene.cycles.transparent_max_bounces=128
  for mode in ('with-approved-trees','ground-only'):
   for o in trees:o.hide_render=mode=='ground-only'
   render_views(scene.name,views,OUT/mode,modes=('textured',),width=640)
  sheet=Image.new('RGB',(1280,1280),'#333333')
  for row,mode in enumerate(('with-approved-trees','ground-only')):
   for col,i in enumerate((0,4)):
    im=Image.open(OUT/mode/f'view-{i}-textured.png').convert('RGBA');bg=Image.new('RGBA',im.size,'#333333');bg.alpha_composite(im);sheet.paste(bg.convert('RGB'),(col*640,row*640))
  sheet.save(OUT/'source-opposite-ground-comparison.png');assert all(sha(Path(p))==h for p,h in guards.items());(OUT/'receipt.json').write_text(json.dumps(dict(status='DIAGNOSTIC only; retained painted foliage, no synthesis',protected_sources=guards,tree_mesh_count=len(trees),native_view_index=0,views=[0,4],top_row='Approved physical trees with current terrain',bottom_row='Same cameras and terrain with trees hidden',scene_saved=False,limits=['Original approved Blender materials provide contact diagnostic, not canonical WebGL renderer equivalence.','Terrain reconstructed from current exact ground quad/image; no pixel altered.','No membership, animation or terrain appearance approval implied.']),indent=2)+'\n')
 finally:release()
if __name__=='__main__':main()
