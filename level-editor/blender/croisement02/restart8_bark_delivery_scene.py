"""Normalize only the sole reusable scene name, preserving every binary byte."""
from pathlib import Path
import copy,hashlib,json,struct,sys
ROOT=Path(__file__).resolve().parents[3]
BASE=ROOT/'level-editor/work/croisement02-refinement/restart8-five-bark-approved-export-v1'
def digest(b):return hashlib.sha256(b).hexdigest()
def unpack(b):
 assert b[:4]==b'glTF' and struct.unpack_from('<I',b,8)[0]==len(b)
 length,kind=struct.unpack_from('<II',b,12);assert kind==0x4e4f534a
 return json.loads(b[20:20+length]),b[20+length:]
def main(n):
 base=BASE/f'tree-{n}-v1';src=base/'delivery-v2';out=base/'delivery-v3';out.mkdir(exist_ok=False);rows=[]
 for name in ['model.glb','lossless.glb']:
  old=(src/name).read_bytes();document,tail=unpack(old);assert len(document['scenes'])==1 and document.get('scene',0)==0
  updated=copy.deepcopy(document);updated['scenes'][0]['name']='default';encoded=json.dumps(updated,separators=(',',':')).encode();encoded+=b' '*(-len(encoded)%4)
  new=struct.pack('<4sII',b'glTF',2,20+len(encoded)+len(tail))+struct.pack('<II',len(encoded),0x4e4f534a)+encoded+tail
  check,binary=unpack(new);assert binary==tail;check['scenes'][0]['name']=document['scenes'][0].get('name');assert check==document
  (out/name).write_bytes(new);rows.append(dict(file=name,source_sha256=digest(old),output_sha256=digest(new),binary_chunks_exact=True,all_other_json_exact=True,original_scene=document['scenes'][0].get('name'),scene='default'))
 descriptor=json.loads((src/'asset.json').read_text());descriptor['model_scene']='default';(out/'asset.json').write_text(json.dumps(descriptor,indent=2)+'\n')
 receipt=dict(status='PASS sole scene name normalization',model_scene='default',files=rows,geometry_uv_images_materials_unchanged=True,source_inputs_unchanged=True,publication=False)
 (base/'scene-name-guard.json').write_text(json.dumps(receipt,indent=2)+'\n');print(n,rows[-1]['output_sha256'])
if __name__=='__main__':main(int(sys.argv[1]))
