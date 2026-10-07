"""Create a private, alpha-zero-only padding derivative of endpoint textures."""
from pathlib import Path
import hashlib,io,json,struct
import numpy as np
from PIL import Image
from scipy.ndimage import distance_transform_edt
ROOT=Path(__file__).resolve().parents[3]
BASE=ROOT/'level-editor/work/croisement02-refinement/restart10-hole-aperture'
source=BASE/'export-v1/receivers-and-endpoints.glb'
out=BASE/'export-padding-v2';out.mkdir(exist_ok=True)
sha=lambda data:hashlib.sha256(data).hexdigest()
raw=source.read_bytes();assert sha(raw)=='c025e0ea2b20530edea3d8a936a8a43e86eb98c6d8d14cd7a37255e3586bf4d6'
jlen=struct.unpack_from('<I',raw,12)[0];doc=json.loads(raw[20:20+jlen]);original=json.loads(json.dumps(doc));binstart=28+jlen;payload=bytearray(raw[binstart:]);before=bytes(payload);guards=[]
for idx in (7,8):
 image=doc['images'][idx];view=doc['bufferViews'][image['bufferView']];start=view.get('byteOffset',0);png=before[start:start+view['byteLength']]
 a=np.array(Image.open(io.BytesIO(png)).convert('RGBA'));known=a[:,:,3]>0;assert known.any();_,nearest=distance_transform_edt(~known,return_indices=True);b=a.copy();b[~known,:3]=a[nearest[0][~known],nearest[1][~known],:3]
 assert np.array_equal(a[:,:,3],b[:,:,3]) and np.array_equal(a[known],b[known])
 stream=io.BytesIO();Image.fromarray(b).save(stream,format='PNG');encoded=stream.getvalue();payload.extend(b'\0'*((-len(payload))%4));offset=len(payload);payload.extend(encoded)
 image['bufferView']=len(doc['bufferViews']);doc['bufferViews'].append({'buffer':0,'byteOffset':offset,'byteLength':len(encoded)})
 guards.append({'image':idx,'name':image['name'],'source_png_sha256':sha(png),'padded_png_sha256':sha(encoded),'changed_alpha_zero_texels':int(np.any(a!=b,axis=2).sum()),'alpha_positive_texels_preserved':int(known.sum()),'all_alpha_exact':True})
 Image.fromarray(b).save(out/(image['name']+'-padded.png'))
assert bytes(payload[:len(before)])==before
for key in original:
 if key not in ('images','bufferViews','buffers'):assert original[key]==doc[key],key
assert doc['images'][:7]==original['images'][:7]
doc['buffers'][0]['byteLength']=len(payload);js=json.dumps(doc,separators=(',',':')).encode();js+=b' '*((-len(js))%4);payload.extend(b'\0'*((-len(payload))%4));result=struct.pack('<III',0x46546c67,2,28+len(js)+len(payload))+struct.pack('<II',len(js),0x4e4f534a)+js+struct.pack('<II',len(payload),0x004e4942)+payload
model=out/source.name;model.write_bytes(result)
report={'status':'PASS','model':str(model),'model_sha256':sha(result),'source_model':str(source),'source_sha256':sha(raw),'images':guards,'all_original_buffer_bytes_exact':True,'all_geometry_uv_nodes_materials_samplers_exact':True,'scope':'Only RGB at fully transparent endpoint texels is nearest-source padded. Every alpha value and alpha-positive RGBA remains exact; no terrain texture changes. Private appearance candidate.'}
(out/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2))
