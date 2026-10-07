"""Verify source triangles and derive explicit unlit RGBA glTF appearance."""
import itertools, json, struct, sys
from pathlib import Path
import bpy
import numpy as np
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
from restart25_export_approved_scatter import BASE, MODEL, MANIFEST, RECEIPT, EXPECTED, sha, pixel_digest, acquire, release
SOURCE=BASE/'restart25-approved-state-materialization-v1/scatter-export-v1'
DEST=SOURCE.with_name('scatter-export-v2')

def main():
 assert sha(RECEIPT)==EXPECTED
 report=json.loads((SOURCE/'report.json').read_text());assert sha(MODEL)==report['approval']['member']['model_sha256']
 data=(SOURCE/'scatter.glb').read_bytes();assert sha(SOURCE/'scatter.glb')==report['model_sha256']
 length=struct.unpack_from('<I',data,12)[0];doc=json.loads(data[20:20+length]);bins=data[20+length:];binary=bins[8:]
 def accessor(index):
  a=doc['accessors'][index];v=doc['bufferViews'][a['bufferView']];dt={5126:'<f4',5125:'<u4',5123:'<u2',5121:'u1'}[a['componentType']];width={'SCALAR':1,'VEC2':2,'VEC3':3}[a['type']]
  assert not a.get('sparse') and not v.get('byteStride')
  return np.frombuffer(binary,dtype=dt,count=a['count']*width,offset=v.get('byteOffset',0)+a.get('byteOffset',0)).reshape(a['count'],width)
 bpy.ops.wm.open_mainfile(filepath=str(MODEL));bpy.context.view_layer.update();proof=[];materials={}
 for node in doc['nodes']:
  assert all(k not in node for k in ['matrix','translation','rotation','scale']), 'This scoped worker requires identity glTF nodes'
  if 'mesh' not in node:continue
  ob=bpy.data.objects[node['name']];mesh=ob.data;mesh.calc_loop_triangles();exported=[]
  for prim in doc['meshes'][node['mesh']]['primitives']:
   p=accessor(prim['attributes']['POSITION']);p=p[:,[0,2,1]]*np.array([1,-1,1]);uv=accessor(prim['attributes']['TEXCOORD_0']).copy();uv[:,1]=1-uv[:,1]
   indices=accessor(prim['indices']).reshape(-1,3);material=doc['materials'][prim['material']];materials[prim['material']]=ob.data.materials[material['name']]
   for tri in indices:exported.append((np.concatenate([p[tri],uv[tri]],axis=1),material['name']))
  assert len(exported)==len(mesh.loop_triangles)
  max_error=0
  for source,(target,name) in zip(mesh.loop_triangles,exported):
   expected=np.array([list(ob.matrix_world@mesh.vertices[mesh.loops[i].vertex_index].co)+list(mesh.uv_layers.active.data[i].uv) for i in source.loops])
   assert mesh.materials[source.material_index].name==name
   error=min(float(np.max(np.abs(expected-target[list(order)]))) for order in itertools.permutations(range(3)))
   assert error<=1e-6,(node['name'],error);max_error=max(max_error,error)
  proof.append({'object':node['name'],'triangles':len(exported),'maximum_world_xyz_uv_error':max_error})
 # The source shader is precisely a texture-alpha blend of transparency and emission.
 # Its equivalent glTF representation is unlit base RGBA with alpha blending.
 material_proof=[]
 for index,source in materials.items():
  nodes=source.node_tree.nodes;links=source.node_tree.links
  kinds=sorted(n.type for n in nodes);assert kinds==sorted(['TEX_IMAGE','EMISSION','BSDF_TRANSPARENT','MIX_SHADER','OUTPUT_MATERIAL'])
  tex=next(n for n in nodes if n.type=='TEX_IMAGE');em=next(n for n in nodes if n.type=='EMISSION');mix=next(n for n in nodes if n.type=='MIX_SHADER')
  assert tex.interpolation=='Closest' and tex.extension=='REPEAT'
  assert mix.inputs[0].links[0].from_socket==tex.outputs['Alpha']
  assert mix.inputs[1].links[0].from_node.type=='BSDF_TRANSPARENT' and mix.inputs[2].links[0].from_node==em
  assert em.inputs['Color'].links[0].from_socket==tex.outputs['Color'] and em.inputs['Strength'].default_value==1
  mat=doc['materials'][index];texture=mat['emissiveTexture']['index'];im=doc['images'][doc['textures'][texture]['source']];bv=doc['bufferViews'][im['bufferView']];start=bv.get('byteOffset',0)
  assert pixel_digest(binary[start:start+bv['byteLength']])==pixel_digest(bytes(tex.image.packed_file.data))
  sampler=doc['samplers'][doc['textures'][texture]['sampler']];assert sampler['magFilter']==9728 and sampler.get('wrapS',10497)==sampler.get('wrapT',10497)==10497
  mat.clear();mat.update(name=source.name,doubleSided=True,alphaMode='BLEND',pbrMetallicRoughness={'baseColorFactor':[1,1,1,1],'baseColorTexture':{'index':texture},'metallicFactor':0,'roughnessFactor':1},extensions={'KHR_materials_unlit':{}})
  material_proof.append({'material':source.name,'exact_shader_equivalence':True,'rgba_exact':True,'nearest_repeat':True})
 doc['extensionsUsed']=sorted(set(doc.get('extensionsUsed',[]))|{'KHR_materials_unlit'})
 doc['extensionsRequired']=sorted(set(doc.get('extensionsRequired',[]))|{'KHR_materials_unlit'})
 DEST.mkdir(exist_ok=False);target=DEST/'scatter.glb';payload=json.dumps(doc,separators=(',',':')).encode();payload+=b' '*((-len(payload))%4)
 target.write_bytes(struct.pack('<III',0x46546c67,2,20+len(payload)+len(bins))+struct.pack('<II',len(payload),0x4e4f534a)+payload+bins)
 assert sha(MODEL)==report['approval']['member']['model_sha256']
 out={'status':'PRIVATE_CPU_VERIFIED_BROWSER_PENDING','approval':report['approval'],'model':str(target),'model_sha256':sha(target),'parent_report':{'path':str(SOURCE/'report.json'),'sha256':sha(SOURCE/'report.json')},'source_model_unchanged':True,'binary_geometry_and_images_unchanged':True,'source_triangle_uv_checks':proof,'materials':material_proof,'scenes':report['scenes'],'instances':report['instances'],'absent_applied':report['absent_applied'],'scope':report['scope'],'correction':'Export-v1 omitted alpha for the supported source emission/transparent mix. Explicit unlit RGBA is mathematically equivalent; no source pixels, mesh geometry, UVs, placement or approval scopes changed.'}
 (DEST/'report.json').write_text(json.dumps(out,indent=2)+'\n');print(json.dumps({'status':out['status'],'model_sha256':out['model_sha256'],'objects':len(proof),'materials':len(material_proof),'instances':out['instances']}),flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
