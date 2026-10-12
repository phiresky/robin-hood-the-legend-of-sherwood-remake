"""Export exact rigid source poses and periodic inferred flight depth as core glTF tracks."""
import json,struct,hashlib,math
from pathlib import Path
import numpy as np
from scipy.spatial import ConvexHull
from scipy.spatial.transform import Rotation
from scipy.interpolate import CubicSpline
from restart21_butterfly07_geometry_v2 import WING,BODY,HINGES
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/active/croisement02'
FIT=BASE/'restart64-butterfly07-pose-refinement-v1/fit.json';ATLAS=BASE/'restart65-butterfly07-exact-observation-atlas-v1'
DEPTH=BASE/'restart14-butterflies/butterfly07-v3-global-corridor-v2-actual-audit/report.json'
OUT=BASE/'restart66-butterfly07-fullpath-export-v3'
BODY_ATLAS=BASE/'restart66-butterfly07-body-observations-v1/body.png'
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
S=math.sin(math.radians(35));C=math.cos(math.radians(35));B=np.array([[1,0,0],[0,-S,-C],[0,-C,S]]);E=np.array([[1,0,0],[0,0,1],[0,-1,0]]);Q=E@B

def main():
 assert not OUT.exists();OUT.mkdir();fit=json.loads(FIT.read_text());d=json.loads(DEPTH.read_text());curve=d['continuous_height_curve'];depth=CubicSpline(curve['times'],curve['heights'],bc_type='periodic');blob=bytearray();doc=dict(asset=dict(version='2.0',generator='Reviewed butterfly rigid motion exporter'),scene=0,scenes=[dict(nodes=[0])],nodes=[],meshes=[],materials=[],images=[],textures=[],samplers=[dict(magFilter=9728,minFilter=9728,wrapS=33071,wrapT=33071)],bufferViews=[],accessors=[],animations=[],extensionsUsed=['KHR_materials_unlit'])
 def raw(data):
  while len(blob)%4:blob.append(0)
  offset=len(blob);blob.extend(data);doc['bufferViews'].append(dict(buffer=0,byteOffset=offset,byteLength=len(data)));return len(doc['bufferViews'])-1
 def accessor(a,kind='VEC3',component=5126):
  a=np.asarray(a,dtype='<f4' if component==5126 else '<u2');view=raw(a.tobytes());r=dict(bufferView=view,componentType=component,count=len(a),type=kind)
  if kind=='SCALAR':r.update(min=[float(a.min())],max=[float(a.max())])
  elif kind=='VEC3':r.update(min=a.min(0).astype(float).tolist(),max=a.max(0).astype(float).tolist())
  doc['accessors'].append(r);return len(doc['accessors'])-1
 for i in range(4):
  view=raw((ATLAS/f'wing-{i}.png').read_bytes());doc['images'].append(dict(bufferView=view,mimeType='image/png'));doc['textures'].append(dict(source=i,sampler=0));doc['materials'].append(dict(name=f'Exact own-source wing {i//2} face {i%2}',pbrMetallicRoughness=dict(baseColorTexture=dict(index=i),metallicFactor=0,roughnessFactor=1),doubleSided=False,extensions={'KHR_materials_unlit':{}}))
 view=raw(BODY_ATLAS.read_bytes());doc['images'].append(dict(bufferView=view,mimeType='image/png'));doc['samplers'].append(dict(magFilter=9728,minFilter=9728,wrapS=10497,wrapT=33071));doc['textures'].append(dict(source=4,sampler=1));doc['materials'].append(dict(name='Own source body observations; gray unknown',pbrMetallicRoughness=dict(baseColorTexture=dict(index=4),metallicFactor=0,roughnessFactor=1),extensions={'KHR_materials_unlit':{}}))
 doc['nodes']=[dict(name='Butterfly07 full inferred flight path',children=[1,2,3]),dict(name='Fixed body',mesh=0)]
 body=BODY@Q.T;hull=ConvexHull(body);faces=hull.simplices.copy()
 for i,f in enumerate(faces):
  a,b,c=body[f]
  if np.dot(np.cross(b-a,c-a),hull.equations[i,:3])<0:faces[i]=f[::-1]
 body_vertices=body[faces].reshape(-1,3);local=BODY[faces];u=np.mod(np.arctan2(local[:,:,2]/.5,local[:,:,0]/.5)/(2*np.pi),1);v=np.arccos(np.clip(local[:,:,1]/2.7,-1,1))/np.pi
 for i in range(len(u)):
  if np.ptp(u[i])>.5:u[i,u[i]<.5]+=1
 body_uv=np.stack([u,v],axis=-1).reshape(-1,2)
 doc['meshes'].append(dict(name='Conserved body',primitives=[dict(attributes=dict(POSITION=accessor(body_vertices),TEXCOORD_0=accessor(body_uv,'VEC2')),indices=accessor(np.arange(len(body_vertices)),'SCALAR',5123),material=4)]))
 for side,sign in enumerate((-1,1)):
  v=WING.copy();v[:,0]*=sign;v=v@Q.T;faces=np.array([[0,i,i+1] for i in range(1,len(v)-1)])
  if np.dot(np.cross(v[faces[0,1]]-v[0],v[faces[0,2]]-v[0]),Q[:,2])<0:faces=faces[:,::-1]
  position=accessor(v);uv=accessor(np.c_[WING[:,0]/8,(WING[:,1]+7)/13],'VEC2');primitives=[]
  for face in range(2):primitives.append(dict(attributes=dict(POSITION=position,TEXCOORD_0=uv),indices=accessor((faces if face==0 else faces[:,::-1]).ravel(),'SCALAR',5123),material=side*2+face))
  doc['meshes'].append(dict(name=f'Conserved wing {side}',primitives=primitives));doc['nodes'].append(dict(name=f'Rigid wing hinge {side}',mesh=side+1,translation=(Q@np.array([HINGES[side],0,0])).tolist()))
 params=np.array([r['parameters'] for r in fit['rows']]);screen=np.array([r['source']['alpha_centroid_display'] for r in fit['rows']]);screen=np.vstack([screen,screen[0]]);regs=np.vstack([params[:,5:7],params[0,5:7]])
 def translation(t,derivative=False,side=1):
  phase=min(int(math.floor(t)),98);fraction=t-phase
  if derivative:
   if float(t).is_integer() and side<0:phase=(int(t)-1)%99
   elif t==99:phase=0
   ds=screen[phase+1]-screen[phase];dr=regs[phase+1]-regs[phase];dz=float(depth(t,1));return E@np.array([ds[0],-(ds[1]+C*dz)/S,dz])+Q@np.r_[dr,0.]
  sc=screen[phase]*(1-fraction)+screen[phase+1]*fraction;reg=regs[phase]*(1-fraction)+regs[phase+1]*fraction;z=float(depth(t));return E@np.array([sc[0],-(sc[1]+C*z)/S,z])+Q@np.r_[reg,0.]
 knots=np.unique(np.r_[curve['times'],np.arange(100)]);times=knots*2/25;values=[]
 for t in knots:values.extend([translation(float(t),True,-1)*12.5,translation(float(t)),translation(float(t),True,1)*12.5])
 animation=dict(name='Butterfly07 reviewed source poses and inferred depth loop',samplers=[],channels=[])
 def track(node,path,times,values,kind,interpolation):
  animation['samplers'].append(dict(input=accessor(times,'SCALAR'),output=accessor(values,kind),interpolation=interpolation));animation['channels'].append(dict(sampler=len(animation['samplers'])-1,target=dict(node=node,path=path)))
 track(0,'translation',times,values,'VEC3','CUBICSPLINE');pt=np.arange(100)*2/25
 for target in range(3):
  quats=[]
  for phase in range(100):
   p=params[phase%99];rotation=Rotation.from_euler('xyz',p[:3],degrees=True).as_matrix() if target==0 else Rotation.from_euler('y',-(-1 if target==1 else 1)*p[2+target],degrees=True).as_matrix();q=Rotation.from_matrix(Q@rotation@Q.T).as_quat()
   if quats and np.dot(quats[-1],q)<0:q=-q
   quats.append(q)
  node=0 if target==0 else target+1
  doc['nodes'][node]['rotation']=quats[0].tolist()
  if target==0:doc['nodes'][node]['translation']=translation(0).tolist()
  track(node,'rotation',pt,quats,'VEC4','LINEAR')
 doc['animations']=[animation];doc['buffers']=[dict(byteLength=len(blob))];text=json.dumps(doc,separators=(',',':')).encode();text+=b' '*((-len(text))%4);blob+=b'\0'*((-len(blob))%4);payload=struct.pack('<III',0x46546c67,2,12+8+len(text)+8+len(blob))+struct.pack('<II',len(text),0x4e4f534a)+text+struct.pack('<II',len(blob),0x004e4942)+blob;model=OUT/'butterfly07-fullpath.glb';model.write_bytes(payload)
 report=dict(status='PRIVATE_FULLPATH_EXPORT_PENDING_IMPORTER_AND_VISUAL_REVIEW',model_sha256=sha(model),fit_sha256=sha(FIT),depth_report_sha256=sha(DEPTH),atlas_report_sha256=sha(ATLAS/'report.json'),cycle_ticks=198,duration_seconds=7.92,translation_keys=len(knots),rotation_keys=100,source_kind='map-animation',source_index=14,coordinate_system='Absolute editor Y-up map space; binding model_origin must cancel native anchor.',interpolation='CUBICSPLINE exact inferred cubic depth plus piecewise-linear source registration/centroid; SLERP body and rigid hinge quaternions. Source holds remain native-only.',limits=['Private unapproved geometry/appearance; no canonical publication.','Body and wing source observations retain gray unknown texels; pixel mixtures are disclosed.','Full-path visual review and importer verification pending; do not reuse discrete saved-model proof as continuous proof.','Contact diagnostic covers same mathematical interpolation on static receivers only.'])
 (OUT/'report.json').write_text(json.dumps(report,indent=2)+'\n');np.savez(OUT/'translation-reference.npz',phase=knots,position=np.array([translation(float(t)) for t in knots]));print(report)
if __name__=='__main__':main()
