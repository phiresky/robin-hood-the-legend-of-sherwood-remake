"""Exercise omission, alpha, ownership and wood-content failures without Blender."""
import copy,hashlib,io,json,struct,tempfile,unittest
from pathlib import Path
import numpy as np
from PIL import Image
from restart2_static_pair_export_guard import validate
class ExportGuardTest(unittest.TestCase):
 def setUp(self):
  self.tmp=tempfile.TemporaryDirectory();self.path=Path(self.tmp.name)/'model.glb';self.doc=dict(asset={'version':'2.0'},bufferViews=[],accessors=[],images=[],textures=[],materials=[],meshes=[]);self.blob=bytearray()
  rgba=np.array([[[130,90,30,255],[90,50,20,0]]],dtype=np.uint8);png=io.BytesIO();Image.fromarray(rgba).save(png,format='PNG');self.png=png.getvalue();view=self.append(self.png);self.doc['images']=[{'bufferView':view,'mimeType':'image/png'}];self.doc['textures']=[{'source':0}];self.expected=[]
  for role,colors in [('dynamic-frame0-provenance',[1,1,1,0,0,0]),('static-native-samples',[1,1,1])]:
   rgba_colors=np.array([[r,1,1,1] for r in colors],dtype='<f4');indices=np.arange(len(colors),dtype='<u4');ci=self.access(rgba_colors,5126,'VEC4');ii=self.access(indices,5125,'SCALAR');index=len(self.doc['materials']);self.doc['materials'].append(dict(name=role,extras={'crown_source_role':role},alphaMode='MASK',alphaCutoff=.5,doubleSided=True,extensions={'KHR_materials_unlit':{}},pbrMetallicRoughness={'baseColorTexture':{'index':0}}));self.doc['meshes'].append({'primitives':[{'material':index,'indices':ii,'attributes':{'COLOR_0':ci}}]});self.expected.append(dict(role=role,triangles=len(colors)//3,native_triangles=1,rgba_sha256=hashlib.sha256(rgba.tobytes()).hexdigest()))
  self.doc['materials'].append(dict(name='wood',alphaMode='MASK',alphaCutoff=.5,emissiveTexture={'index':0},pbrMetallicRoughness={'baseColorTexture':{'index':0}}));self.wood=[dict(complementary=[dict(material='wood',rgb={'packed_sha256':hashlib.sha256(self.png).hexdigest()},mask_true_pixels=1)])]
 def tearDown(self):self.tmp.cleanup()
 def append(self,data):
  self.blob.extend(b'\0'*(-len(self.blob)%4));index=len(self.doc['bufferViews']);self.doc['bufferViews'].append(dict(buffer=0,byteOffset=len(self.blob),byteLength=len(data)));self.blob.extend(data);return index
 def access(self,data,component,kind):
  index=len(self.doc['accessors']);self.doc['accessors'].append(dict(bufferView=self.append(data.tobytes()),componentType=component,count=len(data),type=kind));return index
 def run_guard(self):
  self.doc['buffers']=[{'byteLength':len(self.blob)}];j=json.dumps(self.doc).encode();j+=b' '*(-len(j)%4);blob=bytes(self.blob)+b'\0'*(-len(self.blob)%4);self.path.write_bytes(struct.pack('<III',0x46546c67,2,28+len(j)+len(blob))+struct.pack('<II',len(j),0x4e4f534a)+j+struct.pack('<II',len(blob),0x004e4942)+blob);return validate(self.path,self.expected,self.wood)
 def test_complete_roles_pass(self):self.assertTrue(self.run_guard()['status'].startswith('PASS'))
 def test_missing_static_object_fails(self):
  self.doc['meshes'].pop()
  with self.assertRaises(AssertionError):self.run_guard()
 def test_missing_dynamic_object_fails(self):
  self.doc['meshes'].pop(0)
  with self.assertRaises(AssertionError):self.run_guard()
 def test_source_ownership_erasure_fails(self):
  a=self.doc['accessors'][self.doc['meshes'][1]['primitives'][0]['attributes']['COLOR_0']];offset=self.doc['bufferViews'][a['bufferView']]['byteOffset'];struct.pack_into('<f',self.blob,offset,0)
  with self.assertRaises(AssertionError):self.run_guard()
 def test_alpha_or_rgb_change_fails(self):
  self.expected[1]['rgba_sha256']='0'*64
  with self.assertRaises(AssertionError):self.run_guard()
 def test_wood_rgb_change_fails(self):
  self.wood[0]['complementary'][0]['rgb']['packed_sha256']='0'*64
  with self.assertRaises(AssertionError):self.run_guard()
if __name__=='__main__':unittest.main()
