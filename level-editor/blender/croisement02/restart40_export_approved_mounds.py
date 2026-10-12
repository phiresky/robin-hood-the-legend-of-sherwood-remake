"""Private per-site export of the approved hiding-mound appearances."""
import sys,json,hashlib,struct,shutil,io,time,gc
from PIL import Image
from pathlib import Path
import bpy,numpy as np
HERE=Path(__file__).resolve().parent; ROOT=HERE.parents[2]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender'),str(ROOT/'level-editor/work/active/croisement02/restart2-textures')]
from render_slots import acquire,release
from exact_composite_export_v2 import convert,finalize_document
import lossy_assets as la
BASE=ROOT/'level-editor/work/active/croisement02'
APPROVAL=BASE/'restart3-review-batches/next-tree02-channels-mounds-v1/user-approval.json'
AUTH=BASE/'restart25-approved-state-materialization-v1/mound-ownership-v1/report.json'
DEST=BASE/'restart40-approved-mound-export-v2'
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def image_sha(raw):return hashlib.sha256(Image.open(io.BytesIO(raw)).convert("RGBA").tobytes()).hexdigest()
def main(site):
 approval=json.loads(APPROVAL.read_text());card=next(r for r in approval['decisions']if r['card_id']=='croisement02-hiding-mounds-all-initial-texture');source=Path(card['model']);assert sha(source)==card['model_sha256']=='4facb09a9b79e5996eae1daee2a99a9d294e09541de19d266a3d27be4e98ce52'
 assert sha(AUTH)=='28b8161f8daac462486cdb7dab3b8310e58d5b1d287fd9c192d3616334b4d46f'
 assert shutil.disk_usage(BASE).free>=10*1024**3
 assert int(next(l.split()[1]for l in Path('/proc/meminfo').read_text().splitlines()if l.startswith('MemAvailable:')))*1024>=6*1024**3
 row=next(r for r in json.loads(AUTH.read_text())['records']if r['site']==site);names={r['object']for r in row['objects']};assert len(names)==67
 dest=DEST/site;dest.mkdir(parents=True,exist_ok=False)
 bpy.ops.wm.open_mainfile(filepath=str(source));scene=bpy.context.scene;objects=[scene.objects[n]for n in sorted(names)]
 bpy.data.batch_remove(ids=[o for o in bpy.data.objects if o.name not in names])
 expected={};records=[]
 for o in objects:
  assert not o.modifiers;o.data=o.data.copy();o.data.transform(o.matrix_world);o.matrix_world.identity();o.hide_render=False;o.hide_set(False)
  o.data.calc_loop_triangles();old=list(o.data.loop_triangles);mixed={t.material_index for t in old if any(n.type=='MIX_RGB'for n in o.data.materials[t.material_index].node_tree.nodes)};order=old+[t for t in old if t.material_index in mixed]
  xyz=np.array([[o.data.vertices[v].co[:]for v in t.vertices]for t in order]);uv={u.name:np.array([[u.data[l].uv[:]for l in t.loops]for t in order])for u in o.data.uv_layers}
  rows=convert(o);assert len(rows)==2;records.extend(rows);o.data.calc_loop_triangles();assert np.array_equal(xyz,np.array([[o.data.vertices[v].co[:]for v in t.vertices]for t in o.data.loop_triangles]));assert all(np.array_equal(a,np.array([[o.data.uv_layers[n].data[l].uv[:]for l in t.loops]for t in o.data.loop_triangles]))for n,a in uv.items())
  for i,m in enumerate(o.data.materials):
   ts=[t for t in o.data.loop_triangles if t.material_index==i]
   if not ts:continue
   # Unique material IDs make every primitive independently auditable.
   m=m.copy();o.data.materials[i]=m;key=o.name+'/'+str(i);m['mound_export_key']=key
   p=np.array([[o.data.vertices[v].co[:]for v in t.vertices]for t in ts]);p=p[:,:,[0,2,1]];p[:,:,2]*=-1
   image_hashes={image_sha(bytes(n.image.packed_file.data)) for n in m.node_tree.nodes if n.type=='TEX_IMAGE' and n.image};expected[key]=dict(images=image_hashes,xyz=p,uv={u.name:np.array([[u.data[l].uv[:]for l in t.loops]for t in ts])for u in o.data.uv_layers})
 bpy.context.view_layer.update();bpy.ops.object.select_all(action='DESELECT')
 for o in objects:o.select_set(True)
 bpy.context.view_layer.objects.active=objects[0];target=dest/'model.glb'
 bpy.ops.export_scene.gltf(filepath=str(target),export_format='GLB',use_selection=True,export_extras=True,export_animations=False,export_cameras=False,export_lights=False,export_yup=True,export_apply=False)
 raw=target.read_bytes();length,kind=struct.unpack_from('<II',raw,12);assert kind==0x4e4f534a;doc=json.loads(raw[20:20+length]);tail=raw[20+length:];assert finalize_document(doc)==134;doc['scenes'][0]['name']=site+'-initial';payload=json.dumps(doc,separators=(',',':')).encode();payload+=b' '*(-len(payload)%4);target.write_bytes(struct.pack('<III',0x46546c67,2,20+len(payload)+len(tail))+struct.pack('<II',len(payload),0x4e4f534a)+payload+tail)
 doc,buffers,_=la.read_glb(target);seen=set();checks=[];images=[]
 for im in doc['images']:
  view=doc['bufferViews'][im['bufferView']];start=view.get('byteOffset',0);images.append(image_sha(buffers[view.get('buffer',0)][start:start+view['byteLength']]))
 for node in doc['nodes']:
  if 'mesh'not in node:continue
  assert not any(k in node for k in ('matrix','translation','rotation','scale'))
  for p in doc['meshes'][node['mesh']]['primitives']:
   m=doc['materials'][p['material']];key=m['extras']['mound_export_key'];assert key not in seen;seen.add(key);e=expected[key];ids=la.accessor_array(doc,buffers,p['indices']).reshape(-1);xyz=la.accessor_array(doc,buffers,p['attributes']['POSITION'])[ids].reshape(-1,3,3);error=float(np.max(np.abs(xyz-e['xyz'])));assert error<.0003
   for channel,tex in [('base',m.get('pbrMetallicRoughness',{}).get('baseColorTexture')),('emission',m.get('emissiveTexture'))]:
    if tex is None:continue
    texture=doc['textures'][tex['index']];assert images[texture['source']] in e['images']
    if channel=='base' and m.get('extras',{}).get('export_complementary_binary_ownership'):
     sampler=doc['samplers'][texture['sampler']];assert sampler['magFilter']==9728 and sampler['minFilter']in(9728,9984)
    uv=la.accessor_array(doc,buffers,p['attributes']['TEXCOORD_'+str(tex.get('texCoord',0))])[ids].reshape(-1,3,2);uv[:,:,1]=1-uv[:,:,1];assert min(float(np.max(np.abs(uv-u)))for u in e['uv'].values())<2e-7
   checks.append(dict(key=key,triangles=len(ids)//3,position_max_error=error))
 assert seen==set(expected);assert sha(source)==card['model_sha256']
 report=dict(status='PRIVATE_EXPORT_GEOMETRY_UV_PASS_APPEARANCE_BROWSER_PENDING',site=site,source_model=str(source),source_sha256=sha(source),approval=str(APPROVAL),approval_sha256=sha(APPROVAL),authority_sha256=sha(AUTH),model=str(target),model_sha256=sha(target),model_scene=site+'-initial',coordinate_frame='Absolute approved world coordinates, transformed Z-up to glTF Y-up; no placement translation required.',objects=67,decoded_rgba_exact=True,binary_mask_samplers_nearest=True,conversion_records=records,primitive_guards=checks,canonical_writes=False,recipe_sha256=sha(__file__))
 (dest/'export.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(dict(site=site,model_sha256=sha(target),bytes=target.stat().st_size)),flush=True)
def available_memory():
 return int(next(l.split()[1]for l in Path('/proc/meminfo').read_text().splitlines()if l.startswith('MemAvailable:')))*1024
if __name__=='__main__':
 for site in sys.argv[sys.argv.index('--')+1:]:
  while True:
   while available_memory()<6*1024**3:
    print('Waiting for6GiB available memory before '+site,flush=True);time.sleep(20)
   acquire(slots=2)
   if available_memory()>=6*1024**3:break
   release();print('Memory changed during queue; releasing slot before '+site,flush=True);time.sleep(5)
  try:main(site)
  finally:
   # Release the large loaded worker before waiting for another shared slot.
   bpy.ops.wm.read_factory_settings(use_empty=True);gc.collect();release()
