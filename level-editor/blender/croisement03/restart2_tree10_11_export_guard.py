"""CPU guard for exactly one reserved Arbre06 role and complementary Tree10/11 wood."""
import hashlib,io,json,struct
from pathlib import Path
import numpy as np
from PIL import Image

def read_glb(path):
 raw=Path(path).read_bytes();length=struct.unpack_from('<I',raw,12)[0];doc=json.loads(raw[20:20+length]);return doc,raw[28+length:]

def validate(path,expected,wood):
 doc,blob=read_glb(path)
 def accessor(index):
  a=doc['accessors'][index];v=doc['bufferViews'][a['bufferView']];assert v.get('buffer',0)==0 and 'sparse' not in a
  dtype=np.dtype({5126:'<f4',5123:'<u2',5121:'u1',5125:'<u4'}[a['componentType']]);channels={'SCALAR':1,'VEC2':2,'VEC3':3,'VEC4':4}[a['type']];stride=v.get('byteStride',dtype.itemsize*channels);offset=v.get('byteOffset',0)+a.get('byteOffset',0)
  values=np.ndarray((a['count'],channels),dtype=dtype,buffer=blob,offset=offset,strides=(stride,dtype.itemsize)).copy()
  if a.get('normalized'):values=values.astype(float)/np.iinfo(dtype).max
  return values
 def image_bytes(texture):
  image=doc['images'][doc['textures'][texture['index']]['source']];v=doc['bufferViews'][image['bufferView']];assert v.get('buffer',0)==0;start=v.get('byteOffset',0);return blob[start:start+v['byteLength']]
 expected={r['role']:r for r in expected};assert set(expected)=={'dynamic-frame0-provenance'}
 records={};materials=doc['materials']
 for index,mat in enumerate(materials):
  role=mat.get('extras',{}).get('crown_source_role')
  if not role:continue
  assert role in expected and role not in records,'Missing or duplicated crown material role';e=expected[role]
  assert mat['alphaMode']=='MASK' and mat['alphaCutoff']==.5 and mat['doubleSided'];assert 'KHR_materials_unlit' in mat['extensions']
  texture=mat['pbrMetallicRoughness']['baseColorTexture'];rgba=np.array(Image.open(io.BytesIO(image_bytes(texture))).convert('RGBA'));assert hashlib.sha256(rgba.tobytes()).hexdigest()==e['rgba_sha256'],'Crown physical alpha/RGB changed'
  counts={0:0,1:0};triangles=0
  for mesh in doc['meshes']:
   for primitive in mesh['primitives']:
    if primitive['material']!=index:continue
    assert primitive.get('mode',4)==4 and 'COLOR_0' in primitive['attributes'];colors=accessor(primitive['attributes']['COLOR_0'])[:,0];indices=accessor(primitive['indices']).ravel().reshape(-1,3);values=colors[indices];assert np.all((values==0)|(values==1)) and np.all(values==values[:,:1]),'Physical opacity and source ownership must remain separate binary domains'
    counts[0]+=int((values[:,0]==0).sum());counts[1]+=int((values[:,0]==1).sum());triangles+=len(indices)
  assert triangles==e['triangles'] and counts[1]==e['native_triangles'],'A crown surface or source-owned cell is missing';assert counts[0]==e['triangles']-e['native_triangles'];records[role]=dict(triangles=triangles,native_triangles=counts[1],inferred_triangles=counts[0],rgba_exact=True)
 assert set(records)==set(expected),'Exactly the reserved Arbre06-provenance crown must be exported'
 by_name={m['name']:m for m in materials};wood_records=[]
 for obj in wood:
  for entry in obj['complementary']:
   mat=by_name[entry['material']];assert mat['alphaMode']=='MASK' and mat['alphaCutoff']==.5
   assert hashlib.sha256(image_bytes(mat['emissiveTexture'])).hexdigest()==entry['rgb']['packed_sha256'],'Approved wood RGB resampled or omitted'
   mask=np.array(Image.open(io.BytesIO(image_bytes(mat['pbrMetallicRoughness']['baseColorTexture']))).convert('RGBA'))[:,:,3];assert set(np.unique(mask))<={0,255};assert int(np.count_nonzero(mask))==entry['mask_true_pixels'];wood_records.append(entry['material'])
 assert wood_records,'No approved wood composite exported'
 return dict(status='PASS sole reserved crown role, physical alpha, source ownership and original wood RGB',crowns=records,wood_complementary_materials=wood_records)
