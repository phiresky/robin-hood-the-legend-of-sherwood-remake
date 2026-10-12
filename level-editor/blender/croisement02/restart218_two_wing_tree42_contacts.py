"""Test butterfly sweeps against every independent STEP crown state, without writes to assets."""
import os
os.environ['OPENBLAS_NUM_THREADS']='2';os.environ['OMP_NUM_THREADS']='2'
import argparse,json,struct,hashlib,sys,shutil
from pathlib import Path
import numpy as np
from scipy.spatial.transform import Rotation
import restart14_butterfly07_anatomy_contacts as audit
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release
sha=lambda p:hashlib.file_digest(p.open('rb'),'sha256').hexdigest()
class GLB:
 def __init__(self,p):
  self.path=p;self.raw=p.read_bytes();n=struct.unpack_from('<I',self.raw,12)[0];self.doc=json.loads(self.raw[20:20+n]);self.bin=self.raw[28+n:]
 def acc(self,i):
  a=self.doc['accessors'][i];assert 'sparse' not in a;v=self.doc['bufferViews'][a['bufferView']];bi=v.get('buffer',0);data=(self.path.parent/self.doc['buffers'][bi]['uri']).read_bytes() if 'uri' in self.doc['buffers'][bi] else self.bin;dt=np.dtype({5126:'<f4',5125:'<u4',5123:'<u2',5121:'u1'}[a['componentType']]);w={'SCALAR':1,'VEC2':2,'VEC3':3,'VEC4':4}[a['type']];return np.ndarray((a['count'],w),dtype=dt,buffer=data,offset=v.get('byteOffset',0)+a.get('byteOffset',0),strides=(v.get('byteStride',dt.itemsize*w),dt.itemsize)).copy()
 def image(self,i):
  v=self.doc['bufferViews'][self.doc['images'][i]['bufferView']];o=v.get('byteOffset',0);return self.bin[o:o+v['byteLength']]
def main():
 p=argparse.ArgumentParser();p.add_argument('--candidate',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args();assert not a.output.exists();assert shutil.disk_usage(ROOT).free>=10*1024**3;mem={k:int(v.split()[0])*1024 for k,v in (line.split(':',1) for line in Path('/proc/meminfo').read_text().splitlines())};assert mem['MemAvailable']>=10*1024**3
 mp=ROOT/'level-editor/library/scenes/croisement02.rhlos-map.json';m=json.loads(mp.read_text());src=next(x for x in m['assetSources'] if x['id']=='croisement02-tree-42');static=GLB(ROOT/'level-editor/library'/src['model']);dynamic=GLB(ROOT/'level-editor/library/mission-states/croisement02/reviewed-tree42-motion-v1/model.glb');assert sha(static.path)==src['model_sha256'];sd,dd=static.doc,dynamic.doc
 # Delivery basis, hierarchy and appearance must match the placed static receiver exactly.
 assert sd['nodes']==dd['nodes'];assert sd['scenes']==dd['scenes'];assert sd['materials']==dd['materials'];assert sd.get('textures')==dd.get('textures');assert sd.get('samplers')==dd.get('samplers');assert len(sd['images'])==len(dd['images']);assert all(static.image(i)==dynamic.image(i) for i in range(len(sd['images'])))
 for sm,dm in zip(sd['meshes'],dd['meshes']):
  assert len(sm['primitives'])==len(dm['primitives'])
  for sp,dp in zip(sm['primitives'],dm['primitives']):
   assert sp['attributes'].keys()==dp['attributes'].keys()
   for key in sp['attributes']:assert np.array_equal(static.acc(sp['attributes'][key]),dynamic.acc(dp['attributes'][key]))
   assert np.array_equal(static.acc(sp['indices']),dynamic.acc(dp['indices']));assert sp.get('material')==dp.get('material')
 anim,=dd['animations'];channel,=anim['channels'];assert channel['target']=={'node':0,'path':'weights'};sam=anim['samplers'][channel['sampler']];assert sam['interpolation']=='STEP';times=dynamic.acc(sam['input']).ravel();weights=dynamic.acc(sam['output']).reshape(len(times),-1);assert np.array_equal(weights[0],weights[-1]);states=np.unique(weights,axis=0)
 placed=next(x for x in m['placements'] if x['id']=='croisement02-tree-42');t=placed['transform'];assert t['rot_deg']==0;world=np.eye(4);world[:3,3]=[t['dx'],t['dz'],t['dy']/audit.reader.SIN];transforms={}
 def visit(i,parent):
  node=dd['nodes'][i];local=np.array(node['matrix']).reshape(4,4).T if 'matrix' in node else np.eye(4)
  if 'matrix' not in node:local[:3,:3]=Rotation.from_quat(node.get('rotation',[0,0,0,1])).as_matrix()@np.diag(node.get('scale',[1,1,1]));local[:3,3]=node.get('translation',[0,0,0])
  transforms[i]=parent@local
  for c in node.get('children',[]):visit(c,transforms[i])
 for i in dd['scenes'][dd.get('scene',0)]['nodes']:visit(i,world)
 displacements={};margin=0.
 for pi,prim in enumerate(dd['meshes'][0]['primitives']):
  deltas=np.array([dynamic.acc(target['POSITION']) for target in prim['targets']]);indices=dynamic.acc(prim['indices']).ravel();rows=[]
  for w in states:
   delta=np.einsum('i,ijk->jk',w,deltas)@transforms[0][:3,:3].T;rows.append(delta[indices].reshape(-1,3,3));screen=np.c_[delta[:,0],audit.reader.SIN*delta[:,2]-audit.reader.COS*delta[:,1]];margin=max(margin,float(abs(screen).max()))
  displacements[pi]=rows
 original=audit.reader.main;calls=[]
 def reader_main(*args,**kwargs):
  callback=kwargs['triangle_callback']
  def inspect(placed,node,ni,pi,tri,*rest):
   if ni!=0:return callback(placed,node,ni,pi,tri,*rest)
   for state,delta in enumerate(displacements[pi]):
    owner={**placed,'id':placed['id']+f':independent-state-{state}'};callback(owner,node,ni,pi,tri+delta,*rest);calls.append([pi,state])
  kwargs['triangle_callback']=inspect;kwargs['query_margin']+=margin+1e-6;return original(*args,**kwargs)
 audit.reader.main=reader_main;audit.OUT=a.output;d=json.loads((a.candidate/'inputs.json').read_text());assert sha(a.candidate/'fit.json')==d['joined_fit_sha256'];helper=a.candidate/'geometry.py'
 audit.main(a.candidate/'fit.json',continuous_height_curve=d['continuous_height_curve'],sweep_subdivisions=8,dense_pose_steps=4,geometry_helper=helper,geometry_helper_sha256=sha(helper),receiver_asset_ids=['croisement02-tree-42'],first_witness_only=True,transition_candidates={},expected_map_sha256=sha(mp))
 report={'status':'ALL_INDEPENDENT_STEP_STATES_TESTED','model_sha256':sha(dynamic.path),'static_model_sha256':sha(static.path),'map_sha256':sha(mp),'weights':states.tolist(),'key_times_seconds':times.tolist(),'prefilter_extra_margin':margin,'primitive_state_calls':calls,'basis_equivalence':'All nodes, scene hierarchy, primitive attributes/indices, materials, textures, samplers and embedded images equal static installed receiver. Only additive morph deltas evaluated.','scope':'Every butterfly pose and conservative swept subinterval tested against every distinct STEP state, independent clocks. Contact report determines clearance.'};(a.output/'dynamic-authority.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report),flush=True)
if __name__=='__main__':
 acquire(2)
 try:main()
 finally:release()
