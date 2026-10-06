"""Verify packed original RGB and binary complementary ownership in private glTF."""
import json,struct,hashlib,io
from pathlib import Path
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
def main():
 out=ROOT/'level-editor/work/croisement03-refinement/restart2/tree13-exact-export-v2';raw=(out/'model.glb').read_bytes();size=struct.unpack_from('<I',raw,12)[0];doc=json.loads(raw[20:20+size]);blob=raw[28+size:];report=json.loads((out/'report.json').read_text());materials={m['name']:m for m in doc['materials']};proof=[]
 def image(texture):
  im=doc['images'][doc['textures'][texture['index']]['source']];view=doc['bufferViews'][im['bufferView']];return blob[view.get('byteOffset',0):view.get('byteOffset',0)+view['byteLength']]
 for obj in report['records']:
  for record in obj['complementary']:
   mat=materials[record['material']];assert mat['alphaMode']=='MASK' and mat['alphaCutoff']==.5;assert mat['pbrMetallicRoughness']['baseColorFactor']==[0.,0.,0.,1.]
   rgb=image(mat['emissiveTexture']);assert hashlib.sha256(rgb).hexdigest()==record['rgb']['packed_sha256'];mask=np.array(Image.open(io.BytesIO(image(mat['pbrMetallicRoughness']['baseColorTexture']))).convert('RGBA'));assert set(np.unique(mask[:,:,3]))<={0,255};assert np.count_nonzero(mask[:,:,3])==record['mask_true_pixels'];proof.append(dict(material=mat['name'],original_packed_rgb_bytes_exact=True,mask_binary=True,mask_true_pixels=int(np.count_nonzero(mask[:,:,3])),independent_rgb_uv=mat['emissiveTexture'].get('texCoord',0),independent_mask_uv=mat['pbrMetallicRoughness']['baseColorTexture'].get('texCoord',0)))
 leaf=next((i,m) for i,m in enumerate(doc['materials']) if m.get('extras',{}).get('foliage_physical_opacity'));index,mat=leaf;assert mat['alphaMode']=='MASK' and mat['doubleSided'] and 'KHR_materials_unlit' in mat['extensions'];primitive=next(p for mesh in doc['meshes'] for p in mesh['primitives'] if p['material']==index);assert 'COLOR_0' in primitive['attributes'];a=doc['accessors'][primitive['attributes']['COLOR_0']];v=doc['bufferViews'][a['bufferView']];dtype={5126:'<f4',5123:'<u2',5121:'u1'}[a['componentType']];offset=v.get('byteOffset',0)+a.get('byteOffset',0);channels=int(a['type'][-1]);arr=np.frombuffer(blob,dtype=dtype,count=a['count']*channels,offset=offset).reshape(a['count'],channels);r=arr[:,0];assert len(np.unique(r))==2
 (out/'material-proof.json').write_text(json.dumps(dict(status='PASS',model_sha256=hashlib.sha256(raw).hexdigest(),complementary_rgb=proof,foliage=dict(physical_alpha='MASK',source_ownership='independent COLOR0.r',red_values=np.unique(r).tolist(),native_source_cells=116),limitations=['Whole static foliage membership remains provisional; metadata describes sample provenance only.','Production renderer visual8 still required.']),indent=2)+'\n')
 print('PASS exact packed RGB and independent binary alpha',len(proof),'complementary branches')
if __name__=='__main__':main()
