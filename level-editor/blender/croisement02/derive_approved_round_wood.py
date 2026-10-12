"""Build exact-image lossless delivery models and chained editor previews privately."""
import copy,json,hashlib,sys,shutil
from pathlib import Path
import numpy as np
R=Path.cwd();sys.path[:0]=[str(R/'level-editor/refinement'),str(R/'level-editor/refinement/blender'),str(Path(__file__).parent)]
import lossy_assets as la
from merge_approved_round_wood import pack
B=R/'level-editor/work/active/croisement02/approved-round-wood-integration-oct11-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def image_bytes(path,d,b,im):
 if'uri'in im:return(path.parent/im['uri']).resolve().read_bytes()
 v=d['bufferViews'][im['bufferView']];return b[v.get('buffer',0)][v.get('byteOffset',0):v.get('byteOffset',0)+v['byteLength']]
def main(n):
 assert shutil.disk_usage(R).free>=10*1024**3;assert int(next(x.split()[1]for x in Path('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')))*1024>=6*1024**3;base=B/f'tree-{n}-v1';folder=base/'delivery-v1';source=folder/'model.glb';proof=json.loads((base/'merge-proof.json').read_text());assert sha(source)==proof['model_sha256'];doc,buffers,_=la.read_glb(source);d=copy.deepcopy(doc);body=bytearray();views=[]
 for v in doc['bufferViews']:
  row=copy.deepcopy(v);data=buffers[v.get('buffer',0)][v.get('byteOffset',0):v.get('byteOffset',0)+v['byteLength']];body.extend(b'\0'*(-len(body)%4));row.update(buffer=0,byteOffset=len(body),byteLength=len(data));body.extend(data);views.append(row)
 d['bufferViews']=views;images=[]
 for i,im in enumerate(doc['images']):
  payload=image_bytes(source,doc,buffers,im);images.append(hashlib.sha256(payload).hexdigest())
  if'uri'not in im:continue
  body.extend(b'\0'*(-len(body)%4));d['images'][i]={k:v for k,v in im.items()if k!='uri'};d['images'][i].update(bufferView=len(views),mimeType='image/png'if payload.startswith(b'\x89PNG')else'image/jpeg');views.append(dict(buffer=0,byteOffset=len(body),byteLength=len(payload)));body.extend(payload)
 d['buffers']=[{'byteLength':len(body)}];raw=pack(d,bytes(body));encoded=la.meshopt_bytes(raw,'--encode');target=folder/'lossy.glb';target.write_bytes(encoded);final,fb,_=la.read_glb(target);assert len(final['accessors'])==len(doc['accessors'])
 for i in range(len(doc['accessors'])):assert np.array_equal(la.accessor_array(doc,buffers,i),la.accessor_array(final,fb,i)),i
 assert final['materials']==doc['materials'] and final['nodes']==doc['nodes'] and final.get('samplers')==doc.get('samplers')
 for i,im in enumerate(final['images']):assert hashlib.sha256(image_bytes(target,final,fb,im)).hexdigest()==images[i]
 receipt=dict(asset_id=f'croisement02-tree-{n}',source=sha(source),output=sha(target),settings={'geometry':'lossless meshopt','image_reencoding':False,'alpha':'exact original packed bytes','ownership':'exact original packed bytes'},all_accessor_values_exact=True,all_images_exact=True,materials_nodes_samplers_exact=True,crown_guard=dict(path=str(base/'merge-proof.json'),sha256=sha(base/'merge-proof.json')),publication=False)
 Path(str(target)+'.receipt.json').write_text(json.dumps(receipt,indent=2)+'\n');info=la.write_preview(target,f'croisement02/croisement02-tree-{n}/lossy.glb',folder/'preview.glb');(base/'delivery-proof.json').write_text(json.dumps(dict(status='PASS_LOSSLESS_DELIVERY_AND_CHAINED_PREVIEW',receipt=receipt,preview=info,bytes={'model':source.stat().st_size,'lossy':target.stat().st_size}),indent=2)+'\n');print(n,'PASS',info,flush=True)
if __name__=='__main__':main(int(sys.argv[1]))
