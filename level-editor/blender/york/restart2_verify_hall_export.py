"""Verify private pair exports against approved surface positions, UVs and textures."""
import argparse,hashlib,io,json,struct,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('state');args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
OUT=ROOT/'level-editor/work/york-refinement/restart2/hall-textures-v2/exports-v3'/args.state
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
from PIL import Image
from mathutils import Vector
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
report=json.loads((OUT/'export-report.json').read_text());source=Path(report['approved_model']);assert sha(source)==report['approved_model_sha256']
bpy.ops.wm.open_mainfile(filepath=str(source));bpy.context.window.scene=bpy.data.scenes['york Refinement'];bpy.context.view_layer.update()
rows=[]
for asset in [report['asset_id']]:
 model=OUT/'3d-assets'/asset/'model.glb';raw=model.read_bytes();length=struct.unpack_from('<I',raw,12)[0];doc=json.loads(raw[20:20+length]);binary=raw[28+length:];descriptor=json.loads(model.with_name('asset.json').read_text());pivot=Vector(descriptor['source_origin_scene']);assert list(pivot)==report['preserved_source_pivot']
 def accessor(index):
  a=doc['accessors'][index];view=doc['bufferViews'][a['bufferView']];size={'SCALAR':1,'VEC2':2,'VEC3':3,'VEC4':4}[a['type']];fmt={5126:'f',5125:'I',5123:'H',5121:'B'}[a['componentType']];step=view.get('byteStride',struct.calcsize(fmt)*size);start=view.get('byteOffset',0)+a.get('byteOffset',0)
  return [struct.unpack_from('<'+fmt*size,binary,start+i*step) for i in range(a['count'])]
 for node in doc['nodes']:
  if 'mesh' not in node:continue
  key=node['extras']['source_node'];candidates=[o for o in bpy.context.scene.objects if o.type=='MESH' and not o.hide_render and o.get('asset_group')==asset and o.get('source_node')==key and o.get('projection_component')==node['extras'].get('projection_component')];assert len(candidates)==1;obj=candidates[0];assert not any(k in node for k in ['translation','rotation','scale','matrix'])
  vertices=[obj.matrix_world@v.co-pivot for v in obj.data.vertices];maxerror=0;uv_layers=set();texture_checks=[]
  for primitive in doc['meshes'][node['mesh']]['primitives']:
   positions=accessor(primitive['attributes']['POSITION']);uvs=accessor(primitive['attributes']['TEXCOORD_0']);matched=[]
   for pos in positions:
    index=min(range(len(vertices)),key=lambda i:(vertices[i]-Vector(pos)).length);error=(vertices[index]-Vector(pos)).length;maxerror=max(maxerror,error);assert error<.001;matched.append([i for i,v in enumerate(vertices) if (v-Vector(pos)).length<.001])
   for layer in obj.data.uv_layers:
    lookup={i:[] for i in range(len(vertices))}
    for loop in obj.data.loops:lookup[loop.vertex_index].append(layer.data[loop.index].uv)
    if all(any(abs(u.x-uv[0])<2e-6 and abs(1-u.y-uv[1])<2e-6 for i in indices for u in lookup[i]) for indices,uv in zip(matched,uvs)):uv_layers.add(layer.name)
   assert uv_layers,(asset,key,'UV mismatch')
   material=doc['materials'][primitive['material']];image_index=doc['textures'][material['pbrMetallicRoughness']['baseColorTexture']['index']]['source'];image=doc['images'][image_index];view=doc['bufferViews'][image['bufferView']];png=binary[view.get('byteOffset',0):view.get('byteOffset',0)+view['byteLength']];decoded=Image.open(io.BytesIO(png)).convert('RGBA')
   textures=[n.image for slot in obj.material_slots if slot.material and slot.material.use_nodes for n in slot.material.node_tree.nodes if n.type=='TEX_IMAGE' and n.image]
   match=[]
   for texture in textures:
    if texture.packed_file:
     try:original=Image.open(io.BytesIO(bytes(texture.packed_file.data))).convert('RGBA')
     except Exception:continue
     if original.size==decoded.size and original.tobytes()==decoded.tobytes():match.append(texture.name)
   assert match,(asset,key,'Texture bytes mismatch');texture_checks.extend(match)
  rows.append({'asset_id':asset,'source_node':key,'max_position_error':maxerror,'matching_uv_layers':sorted(uv_layers),'exact_decoded_texture_images':texture_checks})
assert len(rows)==len(report['exported_parts'])
(OUT/'surface-verification.json').write_text(json.dumps({'status':'PASS approved surface vertex positions, exported UVs and exact decoded texture pixels','members':rows,'source_model_sha256':sha(source),'live_integrated':False,'remaining':'Browser and placement validation before promotion'},indent=2)+'\n')
