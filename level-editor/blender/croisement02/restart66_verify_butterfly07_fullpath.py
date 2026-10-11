"""Read exported core glTF tracks and compare dense full-path wing poses independently."""
import json,struct,hashlib,math
from pathlib import Path
import numpy as np
from scipy.interpolate import CubicSpline
from scipy.spatial.transform import Rotation,Slerp
from restart21_butterfly07_geometry_v2 import WING,HINGES
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/croisement02-refinement';OUT=BASE/'restart66-butterfly07-fullpath-export-v3';MODEL=OUT/'butterfly07-fullpath.glb'
FIT=BASE/'restart64-butterfly07-pose-refinement-v1/fit.json'
ATLAS=BASE/'restart65-butterfly07-exact-observation-atlas-v1'
BODY_ATLAS=BASE/'restart66-butterfly07-body-observations-v1/body.png'
def main():
 data=MODEL.read_bytes();length=struct.unpack_from('<I',data,12)[0];doc=json.loads(data[20:20+length]);binary=data[28+length:];fit=json.loads(FIT.read_text());depth_data=json.loads((BASE/'restart14-butterflies/butterfly07-v3-global-corridor-v2-actual-audit/report.json').read_text())['continuous_height_curve'];depth=CubicSpline(depth_data['times'],depth_data['heights'],bc_type='periodic');counts={'SCALAR':1,'VEC2':2,'VEC3':3,'VEC4':4}
 def accessor(index):
  a=doc['accessors'][index];v=doc['bufferViews'][a['bufferView']];return np.frombuffer(binary,dtype='<f4' if a['componentType']==5126 else '<u2',count=a['count']*counts[a['type']],offset=v['byteOffset']).reshape(a['count'],counts[a['type']]).astype(float)
 tracks=[]
 for c in doc['animations'][0]['channels']:
  sampler=doc['animations'][0]['samplers'][c['sampler']];tracks.append((c['target'],sampler,accessor(sampler['input']).ravel(),accessor(sampler['output'])))
 def evaluate(target,sampler,times,values,t):
  i=min(max(np.searchsorted(times,t,side='right')-1,0),len(times)-2);delta=times[i+1]-times[i];u=np.clip((t-times[i])/delta,0,1)
  if sampler['interpolation']=='CUBICSPLINE':return (2*u**3-3*u*u+1)*values[3*i+1]+(u**3-2*u*u+u)*delta*values[3*i+2]+(-2*u**3+3*u*u)*values[3*i+4]+(u**3-u*u)*delta*values[3*i+3]
  return Slerp([0,1],Rotation.from_quat(values[i:i+2]))([u])[0].as_quat()
 s=math.sin(math.radians(35));c=math.cos(math.radians(35));basis=np.array([[1,0,0],[0,-s,-c],[0,-c,s]]);convert=np.array([[1,0,0],[0,0,1],[0,-1,0]]);q=convert@basis;maximum=0;checks=0
 mesh_vertices=[accessor(doc['meshes'][side+1]['primitives'][0]['attributes']['POSITION']) for side in range(2)]
 for phase_time in np.linspace(0,99,3169):
  t=phase_time*2/25;transforms=[dict(position=np.array(n.get('translation',[0,0,0])),rotation=np.array(n.get('rotation',[0,0,0,1]))) for n in doc['nodes']]
  for target,sampler,times,values in tracks:transforms[target['node']]['position' if target['path']=='translation' else 'rotation']=evaluate(target,sampler,times,values,t)
  phase=min(int(phase_time),98);u=phase_time-phase;a=fit['rows'][phase];b=fit['rows'][(phase+1)%99];pa=np.array(a['parameters']);pb=np.array(b['parameters']);p=pa*(1-u)+pb*u;rotation=Slerp([0,1],Rotation.from_euler('xyz',[pa[:3],pb[:3]],degrees=True))([u])[0].as_matrix();screen=np.array(a['source']['alpha_centroid_display'])*(1-u)+np.array(b['source']['alpha_centroid_display'])*u;height=float(depth(phase_time));anchor=np.array([screen[0],-(screen[1]+c*height)/s,height])
  root=transforms[0];rootrot=Rotation.from_quat(root['rotation']).as_matrix()
  for side,sign in enumerate((-1,1)):
   wing=WING.copy();wing[:,0]*=sign;wing=wing@Rotation.from_euler('y',-sign*p[side+3],degrees=True).as_matrix().T;wing[:,0]+=HINGES[side];expected=(anchor+(wing@rotation.T+np.array([p[5],p[6],0]))@basis.T)@convert.T;child=transforms[side+2];actual=(mesh_vertices[side]@Rotation.from_quat(child['rotation']).as_matrix().T+child['position'])@rootrot.T+root['position'];error=float(np.max(np.abs(actual-expected)));maximum=max(maximum,error);checks+=1
   assert error<.001,(phase_time,side,error)
 for domain,image in enumerate(doc['images']):
  view=doc['bufferViews'][image['bufferView']];raw=binary[view['byteOffset']:view['byteOffset']+view['byteLength']];source=ATLAS/f'wing-{domain}.png' if domain<4 else BODY_ATLAS;assert raw==source.read_bytes()
 report=dict(status='PASS_CORE_GLTF_FULLPATH_MOTION_READBACK',model_sha256=hashlib.sha256(data).hexdigest(),fullpath_samples=3169,wing_pose_checks=checks,maximum_vertex_component_error=maximum,embedded_images_exact=True,duration_seconds=7.92,limits=['Independent core glTF mathematical readback, not browser or renderer certification.','Cubic inferred depth and source-centroid path only; dynamic receiver state interactions unproven.','Unapproved source silhouette and body pixel mixtures remain disclosed.']);path=OUT/'readback.json';assert not path.exists();path.write_text(json.dumps(report,indent=2)+'\n');print(report)
if __name__=='__main__':main()
