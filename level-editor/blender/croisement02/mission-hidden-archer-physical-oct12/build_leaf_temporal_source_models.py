"""Map each native frame onto the unchanged bounded physical leaf at its authored pose."""
import sys,json,struct,copy,hashlib,io
from pathlib import Path
import numpy as np
from PIL import Image
HERE=Path(__file__).parent;sys.path.insert(0,str(HERE));import qualify as q
from operational_records import read_record

def main():
 source=q.OUT/'leaf-component-proof-v10/component-proof.glb';raw=source.read_bytes();source_sha=hashlib.sha256(raw).hexdigest();length=struct.unpack_from('<I',raw,12)[0];original=json.loads(raw[20:20+length]);binary=raw[28+length:]
 dest=q.OUT/'leaf-temporal-source-models-v2';assert not dest.exists();dest.mkdir()
 def arr(i):
  a=original['accessors'][i];v=original['bufferViews'][a['bufferView']];dim={'SCALAR':1,'VEC2':2,'VEC3':3}[a['type']];return np.frombuffer(binary,dtype='<f4',count=a['count']*dim,offset=v.get('byteOffset',0)+a.get('byteOffset',0)).reshape(-1,dim)
 primitive=original['meshes'][0]['primitives'][0];base=arr(primitive['attributes']['POSITION']);anim=original['animations'][0];channel=next(c for c in anim['channels']if c['target']['node']==0);sampler=anim['samplers'][channel['sampler']];times=arr(sampler['input']).reshape(-1);weights=arr(sampler['output']).reshape(len(times),-1);assert len(times)==15 and np.array_equal(weights[0],weights[-1])
 frames=read_record(q.OUT/'profile02-native-loop-composition-v1/report.json')['loop_frames'];rows=[]
 for frame,record in enumerate(frames):
  xyz=base.copy()
  for i,w in enumerate(weights[frame]):xyz+=arr(primitive['targets'][i]['POSITION'])*w
  screen=np.c_[xyz[:,0],xyz[:,2]*np.sin(np.deg2rad(35))-xyz[:,1]*np.cos(np.deg2rad(35))];uv=(screen/[1792,1152]).astype('<f4')
  native=q.rgba(record);atlas=Image.new('RGBA',(1792,1152));atlas.alpha_composite(native,tuple(record['top_left']));stream=io.BytesIO();atlas.save(stream,format='PNG');png=stream.getvalue();doc=copy.deepcopy(original);buf=bytearray(binary)
  def blob(data):
   while len(buf)%4:buf.append(0)
   offset=len(buf);buf.extend(data);doc['bufferViews'].append({'buffer':0,'byteOffset':offset,'byteLength':len(data)});return len(doc['bufferViews'])-1
  view=blob(uv.tobytes());doc['accessors'].append({'bufferView':view,'componentType':5126,'count':len(uv),'type':'VEC2'});doc['meshes'][0]['primitives'][0]['attributes']['TEXCOORD_0']=len(doc['accessors'])-1
  view=blob(png);doc['images'].append({'bufferView':view,'mimeType':'image/png'});doc['textures'].append({'source':len(doc['images'])-1,'sampler':len(doc['samplers'])-1});material=doc['materials'][primitive['material']];material['name']=f'Exact native frame{frame} source camera mapping';material['pbrMetallicRoughness']['baseColorTexture']['index']=len(doc['textures'])-1;material['emissiveTexture']['index']=len(doc['textures'])-1
  while len(buf)%4:buf.append(0)
  doc['buffers'][0]['byteLength']=len(buf);payload=json.dumps(doc,separators=(',',':')).encode();payload+=b' '*((-len(payload))%4);path=dest/f'frame-{frame:02}.glb';path.write_bytes(struct.pack('<III',0x46546c67,2,28+len(payload)+len(buf))+struct.pack('<II',len(payload),0x4e4f534a)+payload+struct.pack('<II',len(buf),0x004e4942)+buf)
  assert bytes(buf[:len(binary)])==binary
  rows.append({'frame':frame,'clip_time_seconds':float(times[frame]),'path':str(path),'sha256':hashlib.sha256(path.read_bytes()).hexdigest(),'native_source':record,'native_rgba_sha256':hashlib.sha256(native.tobytes()).hexdigest(),'atlas_rgba_sha256':hashlib.sha256(atlas.tobytes()).hexdigest(),'original_binary_prefix_unchanged':True,'weights':weights[frame].tolist()})
 report={'status':'FOURTEEN_NATIVE_FRAME_MATERIAL_MAPPINGS_ACTUAL_CAPTURE_PENDING','source_model':{'path':str(source),'sha256':source_sha},'method':'Evaluate unchanged authored morph at native clip time; project physical front vertices with original35degree camera, map direct native full-mapRGBA pixels at exact frame anchor. Only source-owner front UV/image changes; complete geometry, morphs, backs and static complement unchanged.','limits':['Each model is qualified only at its named clip time.','Does not claim component source ownership correct until independent native-window and actual renderer proof.','No alpha values invented or trimmed; retains actual native frame transparency.'],'frames':rows}
 (dest/'handoff.json').write_text(json.dumps(report,indent=2)+'\n');print(dest)
if __name__=='__main__':main()
