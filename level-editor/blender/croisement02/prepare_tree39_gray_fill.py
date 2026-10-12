"""Prepare review-only gray-primitive inputs without changing delivery geometry."""
import copy,hashlib,json,math,shutil,struct,sys
from pathlib import Path
import bpy
from mathutils import Vector
R=Path.cwd();sys.path[:0]=[str(R/'level-editor/refinement'),str(R/'level-editor/refinement/blender')]
from render_slots import acquire,release
from refinement_review import render_review,_tile
from render_multiview_asset import render
from PIL import Image
from array import array
B=R/'level-editor/work/active/croisement02';OUT=B/'tree39-gray-fill-input-oct12-v2';A='croisement02-tree-39';LIVE=R/'level-editor/library/3d-assets/croisement02'/A/'model.glb'
EXPECTED='58751ea1cf7068043e608e694a780f491a8473e1c6923a2b318a7b55b7d53b51';PIVOT=(1699,-1110,0);GRAY_COUNT=3;CLOSE=B/'restart6-five-bark-fill-v1/tree-39-fill-v1/shader-restored-v3/close-views.json'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def dump(p,x):p.write_text(json.dumps(x,indent=2)+'\n')
def tile(folder,name):
 images=[]
 for i in range(8):
  im=bpy.data.images.load(str(folder/f'view-{i}-{name}.png'),check_existing=False);v=array('f',[0])*len(im.pixels);im.pixels.foreach_get(v);images.append(v);bpy.data.images.remove(im)
 _tile(images,384,384,folder/f'{name}.png')
def main():
 acquire(slots=2)
 try:
  assert shutil.disk_usage(R).free>=10*1024**3
  assert int(next(x.split()[1]for x in Path('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')))*1024>=6*1024**3
  assert sha(LIVE)==EXPECTED
  OUT.mkdir(exist_ok=False);raw=LIVE.read_bytes();n=struct.unpack_from('<I',raw,12)[0];g=json.loads(raw[20:20+n]);tail=raw[20+n:];before=copy.deepcopy(g);targets=[]
  for ni,node in list(enumerate(g['nodes'])):
   if 'mesh' not in node or 'wood ' not in node.get('name',''):continue
   mesh=g['meshes'][node['mesh']];gray=[p for p in mesh['primitives']if g['materials'][p['material']].get('emissiveFactor')==[0.18000000715255737]*3]
   if not gray:continue
   retained=[p for p in mesh['primitives']if p not in gray];assert len(gray)+len(retained)==len(mesh['primitives'])
   mesh['primitives']=retained;name='Unknown gray '+node['name'];mi=len(g['meshes']);g['meshes'].append({'name':name,'primitives':gray});newnode={k:copy.deepcopy(v)for k,v in node.items()if k not in ['mesh','name','children']};newnode.update(mesh=mi,name=name);newidx=len(g['nodes']);g['nodes'].append(newnode)
   for parent in g['nodes'][:newidx]:
    if ni in parent.get('children',[]):parent['children'].append(newidx)
   targets.append(name)
  for buffer in g.get('buffers',[]):
   if 'uri'in buffer:buffer['uri']=str((LIVE.parent/buffer['uri']).resolve())
  encoded=json.dumps(g,separators=(',',':')).encode();encoded+=b' '*((-len(encoded))%4);(OUT/'scoped-input.glb').write_bytes(struct.pack('<III',0x46546c67,2,20+len(encoded)+len(tail))+struct.pack('<II',len(encoded),0x4e4f534a)+encoded+tail)
  bpy.ops.wm.read_factory_settings(use_empty=True);bpy.ops.import_scene.gltf(filepath=str(OUT/'scoped-input.glb'));scene=bpy.context.scene;scene.name=A+' Gray Fill Input';scene.render.threads_mode='FIXED';scene.render.threads=2;scene.render.engine='CYCLES';scene.cycles.samples=8
  for obj in list(scene.objects):
   if obj.parent is None:obj.location+=Vector(PIVOT)
  col=bpy.data.collections.new(A+' Input Working');scene.collection.children.link(col)
  actual=[];gray=[]
  for obj in list(scene.objects):
   if obj.type!='MESH':continue
   for old in list(obj.users_collection):old.objects.unlink(obj)
   col.objects.link(obj);obj['asset_group']=A;obj['source_node']=obj.name
   if 'Crown' in obj.name:obj.hide_render=True;continue
   actual.append(obj.name)
   if obj.name.startswith('Unknown gray '):gray.append(obj.name)
  assert len(gray)==GRAY_COUNT
  model=OUT/'worker.blend';bpy.ops.wm.save_as_mainfile(filepath=str(model))
  frames=json.loads((CLOSE).read_text());frames.update(scene_name=scene.name,asset_id=A,collection_name=col.name);frames.pop('render_object_names',None);frames['tile_size']=[384,384]
  for view in frames['views']:view['crop']={'width':384,'height':384}
  Image.new('L',(1792,1152)).save(OUT/'empty-source-mask.png');dump(OUT/'mask-inventory.json',{'masks':[{'index':39000,'png':'empty-source-mask.png','box_top_left':[0,0],'box_size':[1792,1152]}]})
  source=Path(frames['source_image']);dump(OUT/'source-masks.json',{'version':1,'mask_inventory':str(OUT/'mask-inventory.json'),'projections':{'exterior':{'source_sha256':sha(source),'state':'Unknown-only exact gray shader branches; no native source ownership asserted','assignments':[{'source_node':name,'mask_indices':[39000],'reviewed':True}for name in actual]}}})
  render_review(OUT/'packet',scene_name=scene.name,collection_name=col.name,asset_id=A,source_path=source,frame_manifest=frames,source_mask_manifest=OUT/'source-masks.json',render_object_names=gray,allow_projection_revision=True,allow_mask_revision=True)
  actualframes=copy.deepcopy(frames);actualframes.pop('render_object_names',None);dump(OUT/'actual-frames.json',actualframes)
  render(OUT/'actual-frames.json',OUT/'context',modes=('textured','solid'),width=384);tile(OUT/'context','textured');tile(OUT/'context','solid')
  # Bind context separately; it is not editable and contributes no source ownership.
  refs=json.loads((B/'restart6-five-bark-fill-v1/tree-39-input-v1/auxiliary-references.json').read_text())['references'];dump(OUT/'supplementary-references.json',{'references':refs,'own_context':str(OUT/'context/textured.png')})
  dump(OUT/'scope.json',{'status':'INPUT_REVIEW_PENDING_NO_API_REQUEST','live_model':str(LIVE),'live_sha256':sha(LIVE),'input_model':str(model),'input_model_sha256':sha(model),'gray_objects':gray,'context_objects':actual,'scope':'Only existing constant-gray wood primitive sets. Mesh primitives are partitioned into separate review objects without editing accessors, UVs, positions, topology, images or binary bytes. Current known material and crown remain protected. Empty source mask deliberately marks gray branches unsupported, not source artwork.','input_packet':str(OUT/'packet/views.json'),'supplementary_context':str(OUT/'context/textured.png'),'permitted_material_references':refs,'first_view':'Original approved camera at index0/top-left','api_authorization':'Existing repository Sunburst/OpenRouter authorization; synthesis must wait for grouped input approval.','geometry_binary_exact':tail==raw[20+n:],'canonical_writes':False})
 finally:release()
if __name__=='__main__':main()
