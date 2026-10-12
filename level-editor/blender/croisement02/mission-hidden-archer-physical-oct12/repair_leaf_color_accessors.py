"""Correct RGB accessor declarations while preserving every binary payload byte."""
import json,struct,hashlib
from pathlib import Path
ROOT=Path(__file__).resolve().parents[4];BASE=ROOT/'level-editor/work/active/croisement02/mission-hidden-archer-physical-oct12-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def repair(source,target):
 raw=source.read_bytes();n=struct.unpack_from('<I',raw,12)[0];doc=json.loads(raw[20:20+n]);tail=raw[20+n:];colors={p['attributes']['COLOR_0']for mesh in doc['meshes']for p in mesh['primitives']if 'COLOR_0'in p['attributes']};fixes=[]
 for i in sorted(colors):
  a=doc['accessors'][i];v=doc['bufferViews'][a['bufferView']]
  assert a['componentType']==5126 and a['type']=='VEC4' and not a.get('byteOffset',0) and 'byteStride'not in v
  assert v['byteLength']==a['count']*3*4,(i,a,v)
  a['type']='VEC3';fixes.append(i)
 for i,a in enumerate(doc['accessors']):
  v=doc['bufferViews'][a['bufferView']];component={5126:4,5125:4,5123:2,5121:1}[a['componentType']];width=component*{'SCALAR':1,'VEC2':2,'VEC3':3,'VEC4':4}[a['type']];required=a.get('byteOffset',0)+(a['count']-1)*v.get('byteStride',width)+width
  assert required<=v['byteLength'],(i,required,v)
  assert v.get('byteOffset',0)+v['byteLength']<=doc['buffers'][0]['byteLength']
 payload=json.dumps(doc,separators=(',',':')).encode();payload+=b' '*((-len(payload))%4);target.parent.mkdir(parents=True,exist_ok=True);assert not target.exists();target.write_bytes(struct.pack('<III',0x46546c67,2,20+len(payload)+len(tail))+struct.pack('<II',len(payload),0x4e4f534a)+payload+tail)
 return {'source':str(source),'source_sha256':sha(source),'path':str(target),'sha256':sha(target),'fixed_color_accessors':fixes,'binary_chunk_sha256':hashlib.sha256(tail).hexdigest(),'binary_payload_unchanged':True,'all_accessor_bounds_pass':True,'interpretation':'RGB color attributes with implicit alpha1; no color bytes invented or removed.'}
def main():
 static=BASE/'leaf-component-proof-v11';r=repair(BASE/'leaf-component-proof-v10/component-proof.glb',static/'component-proof.glb');(static/'repair.json').write_text(json.dumps(r,indent=2)+'\n')
 old=BASE/'leaf-temporal-source-models-v2';new=BASE/'leaf-temporal-source-models-v3';h=json.loads((old/'handoff.json').read_text());reports=[]
 for row in h['frames']:
  fixed=repair(Path(row['path']),new/Path(row['path']).name);reports.append(fixed);row['path']=fixed['path'];row['sha256']=fixed['sha256']
 h['source_model']={'path':str(static/'component-proof.glb'),'sha256':sha(static/'component-proof.glb')};h['accessor_repair']='Only5COLOR_0accessortypes correctedVEC4→VEC3 per exact RGBbuffer lengths; all binarybytes retained. Prior v2failed evidence unchanged.';h['status']='FOURTEEN_PHASE_ACCESSOR_BOUNDS_PASS_GLTFLOADER_CPU_PENDING';(new/'handoff.json').write_text(json.dumps(h,indent=2)+'\n');(new/'repair.json').write_text(json.dumps({'status':'ALL14_ACCESSOR_BOUNDS_PASS_BINARY_UNCHANGED','static':r,'frames':reports},indent=2)+'\n')
if __name__=='__main__':main()
