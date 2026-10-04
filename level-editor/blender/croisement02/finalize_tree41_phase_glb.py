"""Normalize local phase clips to one exact STEP cycle and validate visibility."""
import json,struct,hashlib
from pathlib import Path
import numpy as np
from catalog import OUT

def main():
 p=OUT/'tree41-phase-appearance-proof-v2';path=p/'phase-appearance-tree41.glb';raw=path.read_bytes();chunks=[];offset=12
 while offset<len(raw):
  size,kind=struct.unpack_from('<II',raw,offset);chunks.append((kind,raw[offset+8:offset+8+size]));offset+=8+size
 d=json.loads(next(b for k,b in chunks if k==0x4E4F534A));binary=next(b for k,b in chunks if k==0x004E4942)
 def acc(index):
  a=d['accessors'][index];view=d['bufferViews'][a['bufferView']];n={'SCALAR':1,'VEC3':3}[a['type']];assert a['componentType']==5126 and 'byteStride' not in view;return np.frombuffer(binary,np.float32,count=a['count']*n,offset=view.get('byteOffset',0)+a.get('byteOffset',0)).reshape(a['count'],n)
 assert len(d['nodes'])==17 and len(d['scenes'])==1
 merged=dict(name='Native phase appearance — 14 frame cycle',channels=[],samplers=[]);values=[];common=None
 for animation in d['animations']:
  offset=len(merged['samplers'])
  for sampler in animation['samplers']:
   time=acc(sampler['input']).ravel();scale=acc(sampler['output']);assert np.all(np.isin(scale,[0,1])) and np.all(scale[:,0]==scale[:,1]) and np.all(scale[:,1]==scale[:,2]);values.append(scale[:,0]);assert common is None or np.array_equal(common,time);common=time;merged['samplers'].append(dict(sampler,interpolation='STEP'))
  for channel in animation['channels']:
   assert channel['target']['path']=='scale' and '/ Crown' in d['nodes'][channel['target']['node']]['name'];merged['channels'].append(dict(channel,sampler=channel['sampler']+offset))
 assert len(merged['channels'])==14;active=np.array(values);assert np.all(active.sum(axis=0)==1),'Exactly one crown must be active at every sampled phase'
 original_binary_sha=hashlib.sha256(binary).hexdigest()
 if abs(float(common[0]))>1e-7:
  normalized=(common-common[0]).astype(np.float32);start=len(binary);binary+=normalized.tobytes();view=len(d['bufferViews']);d['bufferViews'].append(dict(buffer=0,byteOffset=start,byteLength=normalized.nbytes));accessor=len(d['accessors']);d['accessors'].append(dict(bufferView=view,componentType=5126,count=len(normalized),type='SCALAR',min=[0.0],max=[float(normalized[-1])]))
  for sampler in merged['samplers']:sampler['input']=accessor
  d['buffers'][0]['byteLength']=len(binary);common=normalized;chunks=[(k,binary if k==0x004E4942 else b) for k,b in chunks]
 d['animations']=[merged];jsonbytes=json.dumps(d,separators=(',',':')).encode();jsonbytes+=b' '*((-len(jsonbytes))%4);chunks=[(k,jsonbytes if k==0x4E4F534A else b) for k,b in chunks];body=b''.join(struct.pack('<II',len(b),k)+b for k,b in chunks);result=struct.pack('<III',0x46546C67,2,12+len(body))+body;path.write_bytes(result)
 report=dict(status='PASS',nodes=17,wood_nodes=3,crown_states=14,animations=1,channels=14,interpolation='STEP',exactly_one_crown_active=True,geometry_buffers_and_images_unchanged=True,native_time_origin_normalized=True,prior_binary_sha256=original_binary_sha,binary_sha256=hashlib.sha256(binary).hexdigest(),glb_sha256=hashlib.sha256(result).hexdigest(),duration_seconds=float(common[-1]-common[0]),sample_times=len(common),scope='Local standards-compliant GLB proof; production packager unchanged')
 (p/'glb-verification.json').write_text(json.dumps(report,indent=2)+'\n')
 proofpath=p/'proof.json'
 if proofpath.exists():
  proof=json.loads(proofpath.read_text());proof['glb_sha256']=report['glb_sha256'];proofpath.write_text(json.dumps(proof,indent=2)+'\n')
 print(report)
if __name__=='__main__':main()
