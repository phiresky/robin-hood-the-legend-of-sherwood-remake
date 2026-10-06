"""Self-contained lossless delivery derivative: exact image bytes and accessor values."""
import sys,json,struct,copy,hashlib
from pathlib import Path
import numpy as np
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
import lossy_assets as la
from render_slots import acquire,release
D=ROOT/'level-editor/work/croisement02-refinement'/__import__('os').environ.get('C02_EXPORT_NAMESPACE','restart7-tree07-approved-export-v1')
def sha(b):return hashlib.sha256(b).hexdigest()
def pack(d,b):
 b+=b'\0'*(-len(b)%4);d['buffers']=[{'byteLength':len(b)}];j=json.dumps(d,separators=(',',':')).encode();j+=b' '*(-len(j)%4);return struct.pack('<4sII',b'glTF',2,28+len(j)+len(b))+struct.pack('<II',len(j),0x4e4f534a)+j+struct.pack('<II',len(b),0x004e4942)+b
def image_bytes(path,d,b,im):
 if 'uri'in im:return(path.parent/im['uri']).resolve().read_bytes()
 v=d['bufferViews'][im['bufferView']];return b[v.get('buffer',0)][v.get('byteOffset',0):v.get('byteOffset',0)+v['byteLength']]
def main():
 src=D/'exact/model.glb';doc,buffers,raw=la.read_glb(src);o=copy.deepcopy(doc);body=bytearray();views=[]
 for v in doc.get('bufferViews',[]):
  c=copy.deepcopy(v);data=buffers[v.get('buffer',0)][v.get('byteOffset',0):v.get('byteOffset',0)+v['byteLength']];body.extend(b'\0'*(-len(body)%4));c.update(buffer=0,byteOffset=len(body),byteLength=len(data));body.extend(data);views.append(c)
 o['bufferViews']=views;images=[]
 for i,im in enumerate(doc.get('images',[])):
  payload=image_bytes(src,doc,buffers,im);images.append(dict(index=i,name=im.get('name'),sha256=sha(payload),bytes=len(payload)))
  if 'bufferView'in im:continue
  body.extend(b'\0'*(-len(body)%4));idx=len(views);views.append(dict(buffer=0,byteOffset=len(body),byteLength=len(payload)));body.extend(payload);o['images'][i]={k:v for k,v in im.items()if k not in ['uri','bufferView']};o['images'][i].update(bufferView=idx,mimeType='image/png'if payload.startswith(b'\x89PNG')else'image/jpeg')
 target=D/'delivery-v2';target.mkdir(exist_ok=False);exact=pack(o,bytes(body));(target/'model.glb').write_bytes(exact);encoded=la.meshopt_bytes(exact,'--encode');(target/'lossless.glb').write_bytes(encoded);final,fb,_=la.read_glb(target/'lossless.glb');assert final['materials']==o['materials']and final['nodes']==o['nodes']and final.get('samplers')==o.get('samplers');assert len(final['accessors'])==len(doc['accessors'])
 for i in range(len(doc['accessors'])):assert np.array_equal(la.accessor_array(doc,buffers,i),la.accessor_array(final,fb,i)),i
 for i,im in enumerate(final['images']):assert 'uri'not in im and sha(image_bytes(target/'lossless.glb',final,fb,im))==images[i]['sha256']
 descriptor=json.loads((D/'exact/asset.json').read_text());descriptor['model']='lossless.glb';(target/'asset.json').write_text(json.dumps(descriptor,indent=2)+'\n');report=dict(status='PASS structural lossless delivery guards; visual review pending',source_glb_sha256=sha(raw),selfcontained_glb_sha256=sha(exact),delivery_glb_sha256=sha(encoded),all_accessor_values_exact=True,accessor_count=len(doc['accessors']),all_image_bytes_exact=True,images=images,materials_nodes_samplers_exact=True,no_external_resources=True,geometry_reduction=False,image_reencoding=False,bytes=dict(source=len(raw),selfcontained=len(exact),lossless=len(encoded)),publication=False);(D/'delivery-proof.json').write_text(json.dumps(report,indent=2)+'\n');print(report['bytes'])
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
