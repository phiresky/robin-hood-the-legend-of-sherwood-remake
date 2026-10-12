"""Convert approved discrete phase workers, retaining saved images and geometry."""
import sys,json,hashlib,struct,io,time
from pathlib import Path
import bpy,numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[4];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender'),str(Path(__file__).parent)]
from render_slots import acquire,release
import lossy_assets as la
from bake_approved03_04_textures_v1 import geometry
import qualify as q
BASE=q.OUT/'approved03-04-textures-v1/private-phase-integration-v1'
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def rgba(data):
 im=Image.open(io.BytesIO(data)).convert('RGBA');return {'size':list(im.size),'sha256':hashlib.sha256(im.tobytes()).hexdigest()}
def main():
 delta=json.loads((BASE/'phase-integration-delta.json').read_text());out=BASE/'exports-v1';assert not out.exists();out.mkdir();reports=[]
 for profile in delta['profiles']:
  number='03' if 'archer03' in profile['card_id'] else '04'
  for record in profile['models']:
   source=Path(record['path']);assert sha(source)==record['sha256'];bpy.ops.wm.open_mainfile(filepath=str(source));active=[o for o in bpy.context.scene.objects if o.type=='MESH' and o.get('source_phase')==record['phase']];assert active
   before={o.name:geometry(o) for o in active};expected={};source_images={};bpy.ops.object.select_all(action='DESELECT')
   for o in active:
    o.hide_viewport=False;o.hide_render=False;o.hide_set(False);o.select_set(True);o.data.calc_loop_triangles();slots={}
    for slot,mat in enumerate(o.data.materials):
     ts=[t for t in o.data.loop_triangles if t.material_index==slot]
     if not ts:continue
     textures=[n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE' and n.image];assert len(textures)<=1
     xyz=np.array([[tuple(o.matrix_world@o.data.vertices[v].co) for v in t.vertices]for t in ts]);xyz=xyz[:,:,[0,2,1]];xyz[:,:,2]*=-1
     row={'positions':xyz,'material':mat.name}
     if textures:
      node=textures[0];assert node.image.packed_file;data=bytes(node.image.packed_file.data);image=rgba(data);source_images[node.image.name]=image;uvname=node.inputs['Vector'].links[0].from_node.uv_map;uv=np.array([[tuple(o.data.uv_layers[uvname].data[i].uv)for i in t.loops]for t in ts]);uv[:,:,1]=1-uv[:,:,1];row.update(uv=uv,image=image)
     slots[mat.name]=row
    expected[o.name]=slots
   target=out/f'{number}-{record["phase"]}.glb'
   bpy.ops.export_scene.gltf(filepath=str(target),export_format='GLB',use_selection=True,export_extras=True,export_yup=True,export_animations=False,export_cameras=False,export_lights=False,export_apply=False)
   raw=target.read_bytes();n=struct.unpack_from('<I',raw,12)[0];doc=json.loads(raw[20:20+n]);tail=raw[20+n:];normalized=[]
   # A pure emission shader is an unlit image, independent of engine lights.
   for material in doc['materials']:
    if 'emissiveTexture' in material:
     material.setdefault('pbrMetallicRoughness',{})['baseColorTexture']=material['emissiveTexture'];material['pbrMetallicRoughness']['baseColorFactor']=[1,1,1,1];material.setdefault('extensions',{})['KHR_materials_unlit']={};normalized.append(material['name'])
   if normalized and 'KHR_materials_unlit' not in doc.setdefault('extensionsUsed',[]):doc['extensionsUsed'].append('KHR_materials_unlit')
   payload=json.dumps(doc,separators=(',',':')).encode();payload+=b' '*((-len(payload))%4);target.write_bytes(struct.pack('<III',0x46546c67,2,20+len(payload)+len(tail))+struct.pack('<II',len(payload),0x4e4f534a)+payload+tail)
   doc,buffers,_=la.read_glb(target);parents={c:i for i,n in enumerate(doc['nodes']) for c in n.get('children',[])}
   def world(i):return (world(parents[i]) if i in parents else np.eye(4))@q.matrix(doc['nodes'][i])
   actual_images=[]
   for im in doc.get('images',[]):
    v=doc['bufferViews'][im['bufferView']];actual_images.append(rgba(buffers[v.get('buffer',0)][v.get('byteOffset',0):v.get('byteOffset',0)+v['byteLength']]))
   guards=[]
   for ni,node in enumerate(doc['nodes']):
    if 'mesh' not in node:continue
    assert node['name'] in expected
    for prim in doc['meshes'][node['mesh']]['primitives']:
     material=doc['materials'][prim['material']];e=expected[node['name']][material['name']];idx=la.accessor_array(doc,buffers,prim['indices']).reshape(-1);v=la.accessor_array(doc,buffers,prim['attributes']['POSITION'])[idx];xyz=(np.c_[v,np.ones(len(v))]@world(ni).T)[:,:3].reshape(-1,3,3);assert xyz.shape==e['positions'].shape;err=float(np.max(np.abs(xyz-e['positions'])));assert err<0.0003,(node['name'],err)
     row={'object':node['name'],'material':material['name'],'triangles':len(xyz),'max_position_error':err}
     if 'image' in e:
      tx=material['pbrMetallicRoughness']['baseColorTexture'];image=actual_images[doc['textures'][tx['index']]['source']];assert image==e['image'];uv=la.accessor_array(doc,buffers,prim['attributes'][f'TEXCOORD_{tx.get("texCoord",0)}'])[idx].reshape(-1,3,2);erruv=float(np.max(np.abs(uv-e['uv'])));assert erruv<2e-7;row.update(image_rgba_exact=True,max_uv_error=erruv)
     guards.append(row)
   assert {o.name:geometry(o)for o in active}==before and sha(source)==record['sha256']
   report={'status':'APPROVED_PHASE_CONVERSION_NUMERIC_PASS_BROWSER_PENDING','profile':number,'phase':record['phase'],'source':record,'output':{'path':str(target),'sha256':sha(target)},'source_geometry_unchanged':True,'primitive_guards':guards,'unlit_materials':normalized,'source_images':source_images,'canonical_writes':False};(out/f'{number}-{record["phase"]}.json').write_text(json.dumps(report,indent=2)+'\n');reports.append(report);print('PHASE_EXPORTED',number,record['phase'],flush=True)
 (out/'summary.json').write_text(json.dumps({'status':'EIGHT_PHASE_CONVERSIONS_PASS_BROWSER_PENDING','phases':reports},indent=2)+'\n')
if __name__=='__main__':
 def available():return int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')))*1024
 while available()<6*1024**3:time.sleep(10)
 acquire()
 try:
  assert available()>=4*1024**3
  main()
 finally:release()
