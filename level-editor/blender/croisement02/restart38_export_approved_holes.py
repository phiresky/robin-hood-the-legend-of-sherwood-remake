"""Export approved hole appearances through exact complementary binary materials."""
import sys,json,struct,hashlib,shutil,io
from pathlib import Path
import bpy,numpy as np
from PIL import Image
HERE=Path(__file__).resolve().parent;ROOT=HERE.parents[2]
sys.path[:0]=[str(HERE),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender'),str(ROOT/'level-editor/work/croisement02-refinement/restart2-textures')]
from render_slots import acquire,release
from exact_composite_export_v2 import convert,finalize_document
from bake_texture_candidate import snapshot
import lossy_assets as la
BASE=ROOT/'level-editor/work/croisement02-refinement';DEST=BASE/'restart38-approved-hole-export-v1'
RECEIPT=BASE/'restart3-review-batches/next-ground-storehouse-hole-v1/user-approval-partial.json';RECEIPT_SHA='db55badd3a8f9545ecbf381f6dc6014d344f8a773e573e055e17bfb7eec5a675'
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def image_digest(data):
 im=Image.open(io.BytesIO(data)).convert('RGBA');return dict(size=list(im.size),rgba_sha256=hashlib.sha256(im.tobytes()).hexdigest())
def emit_glb(path,doc,tail):
 payload=json.dumps(doc,separators=(',',':')).encode();payload+=b' '*(-len(payload)%4);path.write_bytes(struct.pack('<III',0x46546c67,2,20+len(payload)+len(tail))+struct.pack('<II',len(payload),0x4e4f534a)+payload+tail)
def main(state):
 assert state in ('initial','applied');assert sha(RECEIPT)==RECEIPT_SHA
 card=next(c for c in json.loads(RECEIPT.read_text())['approved_cards'] if c['card_id']=='croisement02-hole-endpoint-pair-texture');member=next(m for m in card['members'] if m['asset_id']=='croisement02-hole-'+state);source=Path(member['model']);assert sha(source)==member['model_sha256']
 assert shutil.disk_usage(BASE).free>=10*1024**3
 assert int(next(l.split()[1]for l in Path('/proc/meminfo').read_text().splitlines()if l.startswith('MemAvailable:')))*1024>=6*1024**3
 dest=DEST/state;dest.mkdir(parents=True,exist_ok=False);bpy.ops.wm.open_mainfile(filepath=str(source));scene=bpy.context.scene;objects=[o for o in scene.objects if o.type=='MESH'];assert len(objects)==1;obj=objects[0];before=snapshot(scene,{obj.name});original_images={}
 for m in obj.data.materials:
  if m and m.use_nodes:
   for n in m.node_tree.nodes:
    if n.type=='TEX_IMAGE' and n.image:assert n.image.packed_file;original_images[n.image.name]=image_digest(bytes(n.image.packed_file.data))
 # Bake placement into a private mesh copy, keeping the approved surface fixed.
 obj.data=obj.data.copy();obj.data.transform(obj.matrix_world);obj.matrix_world.identity();bpy.context.view_layer.update();obj.data.calc_loop_triangles();prior=list(obj.data.loop_triangles);mixed={p.material_index for p in prior if any(n.type=='MIX_RGB'for n in obj.data.materials[p.material_index].node_tree.nodes)};order=prior+[t for t in prior if t.material_index in mixed];original_xyz=np.array([[obj.data.vertices[v].co[:]for v in t.vertices]for t in order]);original_uv={u.name:np.array([[u.data[l].uv[:]for l in t.loops]for t in order])for u in obj.data.uv_layers};records=convert(obj);assert len(records)==2;obj.data.calc_loop_triangles();assert np.array_equal(original_xyz,np.array([[obj.data.vertices[v].co[:]for v in t.vertices]for t in obj.data.loop_triangles]));assert all(np.array_equal(values,np.array([[obj.data.uv_layers[name].data[l].uv[:]for l in t.loops]for t in obj.data.loop_triangles]))for name,values in original_uv.items())
 expected={};obj.data.calc_loop_triangles()
 for slot,mat in enumerate(obj.data.materials):
  triangles=[t for t in obj.data.loop_triangles if t.material_index==slot]
  if not triangles:continue
  mat['hole_export_slot']=slot;xyz=np.array([[obj.data.vertices[v].co[:]for v in t.vertices]for t in triangles]);xyz=xyz[:,:,[0,2,1]];xyz[:,:,2]*=-1
  expected[slot]=dict(xyz=xyz,uv={uv.name:np.array([[uv.data[l].uv[:]for l in t.loops]for t in triangles])for uv in obj.data.uv_layers})
 for ob in list(bpy.data.objects):
  if ob!=obj:bpy.data.objects.remove(ob,do_unlink=True)
 obj.hide_render=False;obj.hide_set(False);bpy.ops.object.select_all(action='DESELECT');obj.select_set(True);bpy.context.view_layer.objects.active=obj;bpy.context.preferences.filepaths.save_version=0
 derivative=dest/'worker.blend';bpy.ops.wm.save_as_mainfile(filepath=str(derivative));target=dest/'model.glb';bpy.ops.export_scene.gltf(filepath=str(target),export_format='GLB',use_selection=True,export_extras=True,export_animations=False,export_cameras=False,export_lights=False,export_yup=True,export_apply=False)
 raw=target.read_bytes();length,kind=struct.unpack_from('<II',raw,12);assert kind==0x4e4f534a;doc=json.loads(raw[20:20+length]);tail=raw[20+length:];count=finalize_document(doc);assert count==2;assert len(doc['scenes'])==1;doc['scenes'][0]['name']='endpoint-'+state;emit_glb(target,doc,tail)
 doc,buffers,_=la.read_glb(target);nodes=[n for n in doc['nodes']if 'mesh'in n];assert len(nodes)==1;assert all(k not in nodes[0]for k in ('matrix','translation','rotation','scale'));actual_images=[]
 for im in doc['images']:
  v=doc['bufferViews'][im['bufferView']];start=v.get('byteOffset',0);actual_images.append(image_digest(buffers[v.get('buffer',0)][start:start+v['byteLength']]))
 guards=[];seen=set()
 for prim in doc['meshes'][nodes[0]['mesh']]['primitives']:
  mat=doc['materials'][prim['material']];slot=mat['extras']['hole_export_slot'];assert slot in expected and slot not in seen;seen.add(slot);e=expected[slot];ids=la.accessor_array(doc,buffers,prim['indices']).reshape(-1);xyz=la.accessor_array(doc,buffers,prim['attributes']['POSITION'])[ids].reshape(-1,3,3);assert xyz.shape==e['xyz'].shape;error=float(np.max(np.abs(xyz-e['xyz'])));assert error<.0001
  record=next((r for r in records if r['material']==mat['name']),None);textures=[]
  for key,reference in [('emissiveTexture',mat.get('emissiveTexture')),('baseColorTexture',mat.get('pbrMetallicRoughness',{}).get('baseColorTexture'))]:
   if reference is None:continue
   uv=la.accessor_array(doc,buffers,prim['attributes']['TEXCOORD_'+str(reference.get('texCoord',0))])[ids].reshape(-1,3,2);uv[:,:,1]=1-uv[:,:,1];scores={name:float(np.max(np.abs(uv-values)))for name,values in e['uv'].items()};best=min(scores,key=scores.get);assert scores[best]<2e-7
   texture=doc['textures'][reference['index']];image=actual_images[texture['source']];sampler=doc['samplers'][texture['sampler']]
   if record:
    assert mat['alphaMode']=='MASK'and mat['alphaCutoff']==.5
    if key=='emissiveTexture':assert best==record['rgb']['uv'];assert image==original_images[record['rgb']['image']]
    else:assert best==record['mask_uv'];assert sampler['magFilter']==9728 and sampler['minFilter']in(9728,9984)
   textures.append(dict(channel=key,uv=best,uv_error=scores[best],image=image,sampler=sampler))
  guards.append(dict(material=mat['name'],slot=slot,triangles=len(xyz),maximum_position_error=error,textures=textures))
 assert seen==set(expected);assert sha(source)==member['model_sha256']and sha(RECEIPT)==RECEIPT_SHA
 report=dict(status='PRIVATE_APPROVED_EXPORT_NUMERIC_PASS_BROWSER_PENDING',state=state,source_model=str(source),source_model_sha256=sha(source),approval=str(RECEIPT),approval_sha256=RECEIPT_SHA,model=str(target),model_sha256=sha(target),model_scene='endpoint-'+state,derivative=str(derivative),derivative_sha256=sha(derivative),source_geometry=before,conversion_records=records,primitive_guards=guards,source_images=original_images,embedded_images=actual_images,source_worker_unchanged=True,complementary_surface_and_original_uvs_exact=True,recipe_sha256=sha(__file__),converter_sha256=sha(ROOT/'level-editor/work/croisement02-refinement/restart2-textures/exact_composite_export_v2.py'),limitations=['Complementary binary masks preserve the two approved image branches on coincident surfaces; no surface displacement.','Private export only; browser appearance, receiver aperture and binding integration remain separate.'])
 (dest/'export.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(dict(state=state,model_sha256=sha(target),triangles=sum(r['triangles']for r in guards))),flush=True)
if __name__=='__main__':
 acquire()
 try:main(sys.argv[sys.argv.index('--')+1])
 finally:release()
