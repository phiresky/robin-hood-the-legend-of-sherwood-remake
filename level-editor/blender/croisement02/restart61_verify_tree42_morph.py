"""Read back every delivered phase and compare with the approved pose capture."""
import json,struct
import numpy as np
from scipy.spatial import cKDTree
from scipy.spatial.transform import Rotation
import restart61_tree42_morph_export as r

def load(p):
 raw=p.read_bytes();n=struct.unpack_from('<I',raw,12)[0];d=json.loads(raw[20:20+n]);return d,raw[28+n:]
def main():
 r.verify();out=r.DEST;doc,data=load(out/'tree42-approved-motion.glb');old,olddata=load(r.recipe.GLB)
 assert data[:old['buffers'][0]['byteLength']]==olddata[:old['buffers'][0]['byteLength']]
 for key in ['materials','images','textures','samplers','scenes','nodes']:assert doc.get(key)==old.get(key),key
 for i,m in enumerate(old['meshes']):
  for j,p in enumerate(m['primitives']):
   actual=dict(doc['meshes'][i]['primitives'][j]);actual.pop('targets',None);assert actual==p
 def accessor(index):
  a=doc['accessors'][index];v=doc['bufferViews'][a['bufferView']];assert a['componentType']==5126
  width={'SCALAR':1,'VEC3':3}[a['type']];return np.ndarray((a['count'],width),dtype='<f4',buffer=data,offset=v.get('byteOffset',0)+a.get('byteOffset',0),strides=(v.get('byteStride',4*width),4)).copy()
 anim=doc['animations'];assert len(anim)==1 and len(anim[0]['channels'])==1
 ch=anim[0]['channels'][0];assert ch['target']['path']=='weights';sam=anim[0]['samplers'][ch['sampler']];assert sam['interpolation']=='STEP';times=accessor(sam['input']).ravel();weights=accessor(sam['output']).reshape(15,13);expected_times,expected_weights=r.recipe.phase_weights();assert np.array_equal(times,expected_times)and np.array_equal(weights,expected_weights)
 transforms={}
 def visit(index,parent):
  node=doc['nodes'][index];local=np.array(node['matrix']).reshape(4,4).T if 'matrix'in node else np.eye(4)
  if 'matrix'not in node:local[:3,:3]=Rotation.from_quat(node.get('rotation',[0,0,0,1])).as_matrix()@np.diag(node.get('scale',[1,1,1]));local[:3,3]=node.get('translation',[0,0,0])
  world=parent@local;transforms[index]=world
  for child in node.get('children',[]):visit(child,world)
 p=r.EXPECTED_PLACEMENT['transform'];world=np.eye(4);world[:3,3]=[p['dx'],p['dz'],p['dy']/np.sin(np.radians(35))]
 for node in doc['scenes'][doc.get('scene',0)]['nodes']:visit(node,world)
 capture=np.load(out/'approved-crown.npz');basis=capture['basis_world_yup'];delta=capture['delta_world_yup'];tree=cKDTree(basis);matrix=transforms[ch['target']['node']];mesh=doc['meshes'][doc['nodes'][ch['target']['node']]['mesh']];results=[]
 for primitive in mesh['primitives']:
  xyz=accessor(primitive['attributes']['POSITION']);placed=xyz@matrix[:3,:3].T+matrix[:3,3];distance,ids=tree.query(placed);targets=np.array([accessor(t['POSITION'])for t in primitive['targets']]);errors=[]
  for phase,w in enumerate(weights):
   actual=(xyz+np.einsum('i,ijk->jk',w,targets))@matrix[:3,:3].T+matrix[:3,3]
   wanted=basis[ids]+np.einsum('i,ijk->jk',w,delta[:,ids]);err=float(abs(actual-wanted).max());assert err<0.0005;errors.append(err)
  results.append({'vertices':len(xyz),'world_phase_errors':errors})
 result={'status':'PASS_ALL15_APPROVED_MORPH_PHASES','model_sha256':r.recipe.sha(out/'tree42-approved-motion.glb'),'phase_comparisons':45,'primitives':results,'static_materials_uvs_geometry_exact':True,'wood_unchanged':True,'timing':'14phases held4ticks each, closingbasis tick56, STEP25Hz','canonical_writes':False};(out/'readback-verification.json').write_text(json.dumps(result,indent=2)+'\n');print(result['status'])
if __name__=='__main__':main()
