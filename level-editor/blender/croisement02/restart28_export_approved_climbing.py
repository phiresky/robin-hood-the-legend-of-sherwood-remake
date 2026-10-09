"""Export approved climbing endpoints with explicit physical foliage semantics."""
import sys,json,hashlib,shutil,io,struct
from pathlib import Path
import bpy,numpy as np
from PIL import Image
HERE=Path(__file__).resolve().parent;ROOT=HERE.parents[2]
sys.path[:0]=[str(HERE),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from restart24_bake_climbing_donors import retained
from bake_texture_candidate import snapshot
from export_editor import foliage_export_meshes,finalize_foliage_glb
import lossy_assets as la
BASE=ROOT/'level-editor/work/croisement02-refinement';DEST=BASE/'restart28-approved-climbing-integration-v3'
APPROVAL=BASE/'restart3-review-batches/next-six-textures-channels-v1/user-approval.json';APPROVAL_SHA='7f6abc55fa21eee6ebd3b10c4f079e91e7f8159127a30a23fb9a6955d52e3f9d'
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def digest(data):
 im=Image.open(io.BytesIO(data)).convert('RGBA');return {'size':list(im.size),'rgba_sha256':hashlib.sha256(im.tobytes()).hexdigest()}
def main(state):
 assert sha(APPROVAL)==APPROVAL_SHA
 card=next(r for r in json.loads(APPROVAL.read_text())['decisions'] if r['card_id']=='croisement02-hidden-archer05-climbing-textures');approved=next(r for r in card['members'] if r['state']==state)
 source=Path(approved['model']);assert sha(source)==approved['model_sha256']
 assert shutil.disk_usage(BASE).free>=10*1024**3
 assert int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')))*1024>=6*1024**3
 dest=DEST/state;assert not dest.exists();dest.mkdir(parents=True)
 bpy.ops.wm.open_mainfile(filepath=str(source));bpy.context.view_layer.update();scene=bpy.context.scene;objects=[o for o in scene.objects if o.type=='MESH'];assert len(objects)==1;obj=objects[0];mesh=obj.data;mesh.calc_loop_triangles();before=retained(obj);material_before=snapshot(scene,{obj.name})['physical_foliage']
 assert np.max(np.abs(np.array(obj.matrix_world)-np.eye(4)))<1e-10,'Explicit identity placement required; no implicit transform approximation'
 positions=np.array([v.co[:] for v in mesh.vertices]);positions=positions[:,[0,2,1]];positions[:,2]*=-1
 slots={};images={}
 for i,material in enumerate(mesh.materials):
  if not material:continue
  tex=next(n for n in material.node_tree.nodes if n.type=='TEX_IMAGE');assert tex.image.packed_file;uv_name=tex.inputs['Vector'].links[0].from_node.uv_map
  triangles=[t for t in mesh.loop_triangles if t.material_index==i]
  if not triangles:continue
  xyz=np.array([positions[list(t.vertices)] for t in triangles]);uv=np.array([[mesh.uv_layers[uv_name].data[l].uv[:] for l in t.loops] for t in triangles]);uv[:,:,1]=1-uv[:,:,1]
  ownership=np.array([[mesh.color_attributes['Source ownership'].data[l].color[:] for l in t.loops] for t in triangles]);assert np.all((ownership[:,:,0]==0)|(ownership[:,:,0]==1))
  packed=bytes(tex.image.packed_file.data);image=digest(packed);images[tex.image.name]=image
  slots[i]={'xyz':xyz,'uv':uv,'ownership':ownership,'image':image,'material_name':material.name}
 bpy.ops.object.select_all(action='DESELECT');obj.select_set(True);obj.hide_render=False;obj.hide_set(False)
 target=dest/'model.glb'
 with foliage_export_meshes(objects):
  for slot,material in enumerate(obj.data.materials):
   if material:material['climbing_source_material_slot']=slot
  bpy.ops.export_scene.gltf(filepath=str(target),export_format='GLB',use_selection=True,export_extras=True,export_yup=True,export_animations=False,export_cameras=False,export_lights=False,export_apply=False)
 finalize_foliage_glb(target)
 raw=target.read_bytes();length,kind=struct.unpack_from('<II',raw,12);doc=json.loads(raw[20:20+length]);assert len(doc['scenes'])==1;doc['scenes'][0]['name']='climbing-'+state
 payload=json.dumps(doc,separators=(',',':')).encode();payload+=b' '*((-len(payload))%4);tail=raw[20+length:];target.write_bytes(struct.pack('<III',0x46546c67,2,20+len(payload)+len(tail))+struct.pack('<II',len(payload),kind)+payload+tail)
 doc,buffers,_=la.read_glb(target);nodes=[n for n in doc['nodes'] if 'mesh' in n];assert len(nodes)==1;assert all(k not in nodes[0] for k in ['matrix','translation','rotation','scale'])
 actual_images=[]
 for image in doc['images']:
  view=doc['bufferViews'][image['bufferView']];offset=view.get('byteOffset',0);actual_images.append(digest(buffers[view.get('buffer',0)][offset:offset+view['byteLength']]))
 assert {(tuple(i['size']),i['rgba_sha256']) for i in actual_images}=={(tuple(i['size']),i['rgba_sha256']) for i in images.values()}
 guards=[];seen=set()
 for prim in doc['meshes'][nodes[0]['mesh']]['primitives']:
  material=doc['materials'][prim['material']];slot=material['extras']['climbing_source_material_slot'];assert slot in slots and slot not in seen;seen.add(slot);expected=slots[slot]
  assert material['alphaMode']=='MASK' and material['alphaCutoff']==.5 and material['doubleSided'] is False
  image_ref=material['pbrMetallicRoughness']['baseColorTexture'];image_index=doc['textures'][image_ref['index']]['source'];assert actual_images[image_index]==expected['image']
  indices=la.accessor_array(doc,buffers,prim['indices']).reshape(-1);actual=la.accessor_array(doc,buffers,prim['attributes']['POSITION'])[indices].reshape(-1,3,3);assert actual.shape==expected['xyz'].shape
  error=float(np.max(np.abs(actual-expected['xyz'])));assert error<.0001
  texcoord=image_ref.get('texCoord',0);uv=la.accessor_array(doc,buffers,prim['attributes'][f'TEXCOORD_{texcoord}'])[indices].reshape(-1,3,2);uv_error=float(np.max(np.abs(uv-expected['uv'])));assert uv_error<2e-7
  color=la.accessor_array(doc,buffers,prim['attributes']['COLOR_0'],dequantize=True)[indices];channels=color.shape[1];assert channels in (3,4);color=color.reshape(-1,3,channels);assert np.all(expected['ownership'][:,:,3]==1);assert np.array_equal(color,expected['ownership'][:,:,:channels])
  guards.append({'slot':slot,'triangles':len(actual),'max_position_error':error,'max_uv_error':uv_error,'source_ownership_exact':True,'rgba_exact':True,'one_sided_alpha_mask_exact':True})
 assert seen==set(slots);assert retained(obj)==before and snapshot(scene,{obj.name})['physical_foliage']==material_before
 assert sha(source)==approved['model_sha256'] and sha(APPROVAL)==APPROVAL_SHA
 report={'status':'PRIVATE_APPROVED_EXPORT_NUMERIC_PASS_BROWSER_PENDING','state':state,'source_model':str(source),'source_model_sha256':sha(source),'approval':str(APPROVAL),'approval_sha256':APPROVAL_SHA,'model':str(target),'model_sha256':sha(target),'model_scene':'climbing-'+state,'saved_world_placement':[0,0,0],'source_geometry':before,'primitive_guards':guards,'triangles':sum(r['triangles'] for r in guards),'embedded_images':actual_images,'source_images':images,'source_worker_unchanged':True,'geometry_uv_ownership_alpha_preserved':True,'recipe_sha256':sha(__file__),'limitations':['Native transition remains source artwork; no geometry interpolation is inferred.','No canonical write. Browser material/placement review remains required.']}
 (dest/'export.json').write_text(json.dumps(report,indent=2)+'\n');assert sum(p.stat().st_size for p in DEST.rglob('*') if p.is_file())<80*1024**2
 print(json.dumps({k:report[k] for k in ['status','state','model_sha256','triangles']}),flush=True)
if __name__=='__main__':
 acquire(slots=2)
 try:main(sys.argv[sys.argv.index('--')+1])
 finally:release()
