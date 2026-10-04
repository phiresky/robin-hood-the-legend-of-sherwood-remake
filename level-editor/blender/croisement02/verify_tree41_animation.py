"""Audit standard morph animation and quantify source-phase coverage limits."""
import json,struct
from pathlib import Path
import numpy as np
from scipy.ndimage import binary_dilation
from PIL import Image
from catalog import OUT

def main():
 p=OUT/'tree41-animation-proof';b=(p/'animated-tree41.glb').read_bytes();length=struct.unpack_from('<I',b,12)[0];d=json.loads(b[20:20+length]);offset=20+length;binlen,tag=struct.unpack_from('<II',b,offset);buf=b[offset+8:offset+8+binlen]
 def acc(index):
  a=d['accessors'][index];n={'SCALAR':1,'VEC3':3,'VEC2':2,'VEC4':4}[a['type']];dtype={5126:np.float32,5125:np.uint32,5123:np.uint16,5121:np.uint8}[a['componentType']];view=d['bufferViews'][a['bufferView']];assert 'byteStride' not in view;return np.frombuffer(buf,dtype=dtype,count=a['count']*n,offset=view.get('byteOffset',0)+a.get('byteOffset',0)).reshape(a['count'],n)
 assert len(d['scenes'])==1 and len(d['nodes'])==4
 channels=[c for a in d['animations'] for c in a['channels']];assert len(channels)==1 and channels[0]['target']['path']=='weights'
 c=channels[0];node=d['nodes'][c['target']['node']];assert node['name'].endswith('/ Crown');sampler=d['animations'][0]['samplers'][c['sampler']];weights=acc(sampler['output']).reshape(-1,13);times=acc(sampler['input']).ravel()
 assert len(times)==len(weights)==15 and sampler['interpolation']=='STEP';assert np.max(weights)>0 and np.max(weights[0])==0
 assert np.allclose(np.diff(times),.1);assert all(len(prim['targets'])==13 for prim in d['meshes'][node['mesh']]['primitives'])
 assert all('targets' not in prim for i,m in enumerate(d['meshes']) if i!=node['mesh'] for prim in m['primitives'])
 report=dict(status='PASS',scenes=1,nodes=4,animated_node=node['name'],standard_animation_path='weights',morph_targets=13,interpolation=sampler['interpolation'],time_samples=len(times),duration_seconds=float(times[-1]-times[0]),basephase_weights_zero=True,wood_has_no_morph_targets=True,limitations=['Structural/morph sampling verification; browser playback and material fidelity remain to inspect.'])
 (p/'glb-verification.json').write_text(json.dumps(report,indent=2)+'\n')
 data=np.load(p/'motion.npz');x,y,w,h=data['bbox'];frames=data['alpha']>.5;flows=data['flow'];packet=json.loads((OUT/'forest-v4-sources/tree-41/partition.json').read_text());px,py,pw,ph=packet['native_bbox'];full=Image.open(OUT/'forest-v4-sources/tree-41/complete-source.png').convert('RGBA');owned=np.array(full.crop((x-px,y-py,x-px+w,y-py+h)))[:,:,3]>.5*255
 domain=binary_dilation(owned,iterations=5);first=frames[0]&owned;yy,xx=np.where(first);metrics=[];union=np.zeros_like(first)
 for i,flow in enumerate(flows):
  target=frames[i]&domain;union|=target;splat=np.zeros_like(first);tx=np.clip(np.rint(xx+flow[yy,xx,0]).astype(int),0,w-1);ty=np.clip(np.rint(yy+flow[yy,xx,1]).astype(int),0,h-1);splat[ty,tx]=True
  metrics.append(dict(phase=i,candidate_target_pixels=int(target.sum()),motion_pixels=int(splat.sum()),missed_candidate_pixels=int((target&~splat).sum()),extra_pixels=int((splat&~target).sum()),iou=float((splat&target).sum()/max(1,(splat|target).sum()))))
 visual=np.zeros((h,w,3),np.uint8);visual[first]=[70,120,70];visual[union&~first]=[255,0,255];Image.fromarray(visual).save(p/'native-temporal-union.png')
 hold=dict(status='HOLD_FOR_TEMPORAL_APPEARANCE',method='Source-space forward-splat diagnostic, not a mesh silhouette pass. Temporal candidate ownership is frame alpha inside5px dilation of static crown ownership; adjacent crown attribution remains uncertain.',new_temporal_candidate_pixels=int((union&~first).sum()),phases=metrics,conclusion='Measured geometry motion is real and standard GLB playable, but motion-only fixed alpha/RGB does not reproduce all native phase appearance. This is an isolated proof, not completed animation parity.')
 (p/'temporal-coverage.json').write_text(json.dumps(hold,indent=2)+'\n');print(json.dumps({'glb':report,'temporal_new_pixels':hold['new_temporal_candidate_pixels'],'phase7':metrics[7]}))
if __name__=='__main__':main()
