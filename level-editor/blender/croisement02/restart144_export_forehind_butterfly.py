"""Export four independently articulated wings and separately textured body primitives."""
import json,struct,hashlib,math
from pathlib import Path
import numpy as np
from scipy.spatial import ConvexHull
from scipy.spatial.transform import Rotation
from scipy.interpolate import CubicSpline
import argparse
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/active/croisement02'
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
S=math.sin(math.radians(35));C=math.cos(math.radians(35));B=np.array([[1,0,0],[0,-S,-C],[0,-C,S]]);E=np.array([[1,0,0],[0,0,1],[0,-1,0]]);Q=E@B

def main():
 parser=argparse.ArgumentParser();parser.add_argument('--fit',type=Path,required=True);parser.add_argument('--atlas',type=Path,required=True);parser.add_argument('--depth',type=Path,required=True);parser.add_argument('--output',type=Path,required=True)
 args=parser.parse_args();FIT=args.fit;ATLAS=args.atlas;DEPTH=args.depth;OUT=args.output;BODY_ATLAS=ATLAS/'body.png'
 assert not OUT.exists();OUT.mkdir();fit=json.loads(FIT.read_text());SOURCE_INDEX=fit['animation_index'];label=f'Butterfly{SOURCE_INDEX-7:02}';spec=fit['fixed_geometry'];WING=np.asarray(spec['wing_outline']);RADII=np.ones(3);HINGES=spec['wing_hinge_offsets'];BODY=np.array([[RADII[0]*np.sin(a)*np.cos(t),RADII[1]*np.cos(a),RADII[2]*np.sin(a)*np.sin(t)] for a in np.linspace(0,np.pi,9) for t in np.linspace(0,2*np.pi,17)[:-1]])
 atlas_report=json.loads((ATLAS/'report.json').read_text());assert atlas_report['animation_index']==SOURCE_INDEX;uv_domain=atlas_report['uv_domain'];d=json.loads(DEPTH.read_text());assert atlas_report['fit_sha256']==d['source_fit_sha256'];assert atlas_report['source_frames']==[r['source'] for r in fit['rows']];assert sha(FIT)==d['joined_fit_sha256'];curve=d['continuous_height_curve'];depth=CubicSpline(curve['times'],curve['heights'],bc_type='periodic');blob=bytearray();doc=dict(asset=dict(version='2.0',generator='Private segmented-body butterfly motion exporter'),scene=0,scenes=[dict(nodes=[0])],nodes=[],meshes=[],materials=[],images=[],textures=[],samplers=[dict(magFilter=9728,minFilter=9728,wrapS=33071,wrapT=33071)],bufferViews=[],accessors=[],animations=[],extensionsUsed=['KHR_materials_unlit'])
 def raw(data):
  while len(blob)%4:blob.append(0)
  offset=len(blob);blob.extend(data);doc['bufferViews'].append(dict(buffer=0,byteOffset=offset,byteLength=len(data)));return len(doc['bufferViews'])-1
 def accessor(a,kind='VEC3',component=5126):
  a=np.asarray(a,dtype='<f4' if component==5126 else '<u2');view=raw(a.tobytes());r=dict(bufferView=view,componentType=component,count=len(a),type=kind)
  if kind=='SCALAR':r.update(min=[float(a.min())],max=[float(a.max())])
  elif kind=='VEC3':r.update(min=a.min(0).astype(float).tolist(),max=a.max(0).astype(float).tolist())
  doc['accessors'].append(r);return len(doc['accessors'])-1
 for i in range(8):
  view=raw((ATLAS/f'wing-{i}.png').read_bytes());doc['images'].append(dict(bufferView=view,mimeType='image/png'));doc['textures'].append(dict(source=i,sampler=0));doc['materials'].append(dict(name=f'Exact own-source wing {i//2} face {i%2}',pbrMetallicRoughness=dict(baseColorTexture=dict(index=i),metallicFactor=0,roughnessFactor=1),doubleSided=False,extensions={'KHR_materials_unlit':{}}))
 doc['samplers'].append(dict(magFilter=9728,minFilter=9728,wrapS=10497,wrapT=33071))
 for index,part in enumerate(spec['body_parts']):
  view=raw((ATLAS/(part['name']+'.png')).read_bytes());doc['images'].append(dict(bufferView=view,mimeType='image/png'));doc['textures'].append(dict(source=8+index,sampler=1));doc['materials'].append(dict(name='Own source '+part['name']+' observations; gray unknown',pbrMetallicRoughness=dict(baseColorTexture=dict(index=8+index),metallicFactor=0,roughnessFactor=1),extensions={'KHR_materials_unlit':{}}))
 doc['nodes']=[dict(name=label+' full inferred flight path',children=[1,2,3,4,5]),dict(name='Separate head neck thorax abdomen',mesh=0)]
 primitives=[]
 for index,part in enumerate(spec['body_parts']):
  radii=np.array(part['radii']);center=np.array(part['center']);unit=np.array([[np.sin(a)*np.cos(t),np.cos(a),np.sin(a)*np.sin(t)] for a in np.linspace(0,np.pi,9) for t in np.linspace(0,2*np.pi,17)[:-1]]);local=unit*radii+center;body=local@Q.T;hull=ConvexHull(body);faces=hull.simplices.copy()
  for i,f in enumerate(faces):
   a,b,c=body[f]
   if np.dot(np.cross(b-a,c-a),hull.equations[i,:3])<0:faces[i]=f[::-1]
  vertices=body[faces].reshape(-1,3);surface=unit[faces];u=np.mod(np.arctan2(surface[:,:,2],surface[:,:,0])/(2*np.pi),1);v=np.arccos(np.clip(surface[:,:,1],-1,1))/np.pi
  for i in range(len(u)):
   if np.ptp(u[i])>.5:u[i,u[i]<.5]+=1
  uv=np.stack([u,v],axis=-1).reshape(-1,2);primitives.append(dict(attributes=dict(POSITION=accessor(vertices),TEXCOORD_0=accessor(uv,'VEC2')),indices=accessor(np.arange(len(vertices)),'SCALAR',5123),material=8+index,extras=dict(body_part=part['name'])))
 doc['meshes'].append(dict(name='Separate physical body components',primitives=primitives))
 for side in range(4):
  sign=(-1,1)[side//2];WING=np.asarray(spec['wing_regions'][side%2]);v=WING.copy();v[:,0]*=sign;v=v@Q.T;faces=np.array([[0,i,i+1] for i in range(1,len(v)-1)])
  if np.dot(np.cross(v[faces[0,1]]-v[0],v[faces[0,2]]-v[0]),Q[:,2])<0:faces=faces[:,::-1]
  position=accessor(v);uv=accessor(np.c_[(WING[:,0]-uv_domain[0])/(uv_domain[1]-uv_domain[0]),(WING[:,1]-uv_domain[2])/(uv_domain[3]-uv_domain[2])],'VEC2');primitives=[]
  for face in range(2):primitives.append(dict(attributes=dict(POSITION=position,TEXCOORD_0=uv),indices=accessor((faces if face==0 else faces[:,::-1]).ravel(),'SCALAR',5123),material=side*2+face))
  doc['meshes'].append(dict(name=f'Conserved wing {side}',primitives=primitives));doc['nodes'].append(dict(name=f'Rigid wing hinge {side}',mesh=side+1,translation=(Q@np.array([HINGES[side//2],0,0])).tolist()))
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
 animation=dict(name=label+' own-source poses and inferred depth loop',samplers=[],channels=[])
 def track(node,path,times,values,kind,interpolation):
  animation['samplers'].append(dict(input=accessor(times,'SCALAR'),output=accessor(values,kind),interpolation=interpolation));animation['channels'].append(dict(sampler=len(animation['samplers'])-1,target=dict(node=node,path=path)))
 track(0,'translation',times,values,'VEC3','CUBICSPLINE');pt=np.arange(100)*2/25
 for target in range(5):
  quats=[]
  for phase in range(100):
   p=params[phase%99];rotation=Rotation.from_euler('xyz',p[:3],degrees=True).as_matrix() if target==0 else Rotation.from_euler('y',-(-1 if target<=2 else 1)*p[(3+(target-1)//2) if (target-1)%2==0 else (7+(target-1)//2)],degrees=True).as_matrix();q=Rotation.from_matrix(Q@rotation@Q.T).as_quat()
   if quats and np.dot(quats[-1],q)<0:q=-q
   quats.append(q)
  node=0 if target==0 else target+1
  doc['nodes'][node]['rotation']=quats[0].tolist()
  if target==0:doc['nodes'][node]['translation']=translation(0).tolist()
  track(node,'rotation',pt,quats,'VEC4','LINEAR')
 doc['animations']=[animation];doc['buffers']=[dict(byteLength=len(blob))];text=json.dumps(doc,separators=(',',':')).encode();text+=b' '*((-len(text))%4);blob+=b'\0'*((-len(blob))%4);payload=struct.pack('<III',0x46546c67,2,12+8+len(text)+8+len(blob))+struct.pack('<II',len(text),0x4e4f534a)+text+struct.pack('<II',len(blob),0x004e4942)+blob;model=OUT/(label.lower()+'-fullpath.glb');model.write_bytes(payload)
 report=dict(status='PRIVATE_FULLPATH_EXPORT_PENDING_IMPORTER_AND_VISUAL_REVIEW',model_sha256=sha(model),fit_sha256=sha(FIT),recipe_sha256=sha(Path(__file__)),depth_report_sha256=sha(DEPTH),atlas_report_sha256=sha(ATLAS/'report.json'),cycle_ticks=198,duration_seconds=7.92,translation_keys=len(knots),rotation_keys=100,source_kind='map-animation',source_index=SOURCE_INDEX,coordinate_system='Absolute editor Y-up map space; binding model_origin must cancel native anchor.',interpolation='CUBICSPLINE exact inferred cubic depth plus piecewise-linear source registration/centroid; SLERP body and rigid hinge quaternions. Source holds remain native-only.',limits=['Private unapproved geometry/appearance; no canonical publication.','Body and wing source observations retain gray unknown texels; pixel mixtures are disclosed.','Full-path visual review and importer verification pending; do not reuse discrete saved-model proof as continuous proof.','Inherited depth is a staging placeholder; new four-wing continuous contact audit is required.'])
 (OUT/'report.json').write_text(json.dumps(report,indent=2)+'\n');np.savez(OUT/'translation-reference.npz',phase=knots,position=np.array([translation(float(t)) for t in knots]));print(report)
if __name__=='__main__':main()
