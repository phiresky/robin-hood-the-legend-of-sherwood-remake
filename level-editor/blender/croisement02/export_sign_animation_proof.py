"""Export an isolated sign proof with standard native-timed node animation."""
import json,sys,struct,math
from pathlib import Path
import bpy,numpy as np
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release

def main():
 base=OUT/'state-sign-candidate';source=base/'phase-appearance-v1/model.blend';digest=sha(source);assert digest==json.loads((base/'phase-appearance-v1/evidence.json').read_text())['model_sha256'];dst=base/'animation-export-v2';dst.mkdir(exist_ok=False);bpy.ops.wm.open_mainfile(filepath=str(source));scene=bpy.context.scene;scene.frame_set(1);bpy.ops.object.select_all(action='DESELECT');selected=[o for o in scene.objects if o.type in {'MESH','EMPTY'}]
 for o in selected:o.select_set(True)
 rawpath=dst/'static-source.glb';bpy.ops.export_scene.gltf(filepath=str(rawpath),export_format='GLB',use_selection=True,export_animations=False,export_cameras=False,export_lights=False,export_yup=True,export_extras=True);raw=rawpath.read_bytes();length=struct.unpack_from('<I',raw,12)[0];doc=json.loads(raw[20:20+length]);binary=bytearray(raw[28+length:]);body=[(i,n)for i,n in enumerate(doc['nodes'])if 'native_body_frame'in n.get('extras',{})];shadows=[(i,n)for i,n in enumerate(doc['nodes'])if 'native_frame'in n.get('extras',{})];assert len(body)==64 and len(shadows)==32;root=next((i,n)for i,n in enumerate(doc['nodes'])if n.get('name')=='Rotating sign pivot');angle=scene.objects['Rotating sign pivot'].rotation_euler.z
 expected=[0,math.sin(angle/2),0,math.cos(angle/2)];assert np.allclose(root[1]['rotation'],expected,atol=1e-6)
 def acc(values,kind):
  values=np.asarray(values,dtype=np.float32);binary.extend(b'\0'*(-len(binary)%4));view=len(doc['bufferViews']);doc['bufferViews'].append(dict(buffer=0,byteOffset=len(binary),byteLength=values.nbytes));binary.extend(values.tobytes());index=len(doc['accessors']);a=dict(bufferView=view,componentType=5126,count=len(values),type=kind)
  if kind=='SCALAR':a.update(min=[float(values.min())],max=[float(values.max())])
  doc['accessors'].append(a);return index
 # Explicit standard alpha material preserves the packed native ground mask;
 # the generic shader exporter does not recognize its transparent/emission mix.
 sampler=len(doc.setdefault('samplers',[]));doc['samplers'].append(dict(magFilter=9728,minFilter=9728,wrapS=33071,wrapT=33071));doc.setdefault('extensionsUsed',[]).append('KHR_materials_unlit')
 for _,node in shadows:
  phase=node['extras']['native_frame'];image_path=base/f'painted-shadow-v2/shadow-{phase:02}.png';png=image_path.read_bytes();binary.extend(b'\0'*(-len(binary)%4));view=len(doc['bufferViews']);doc['bufferViews'].append(dict(buffer=0,byteOffset=len(binary),byteLength=len(png)));binary.extend(png);image_index=len(doc['images']);doc['images'].append(dict(bufferView=view,mimeType='image/png',name=f'Native shadow phase {phase:02}'));texture=len(doc['textures']);doc['textures'].append(dict(sampler=sampler,source=image_index));primitives=doc['meshes'][node['mesh']]['primitives'];assert len(primitives)==1;material=doc['materials'][primitives[0]['material']];material.update(alphaMode='MASK',alphaCutoff=.5,doubleSided=True,pbrMetallicRoughness=dict(baseColorFactor=[1,1,1,1],baseColorTexture=dict(index=texture),metallicFactor=0,roughnessFactor=1),extensions={'KHR_materials_unlit':{}})
 times=acc(np.arange(33)*.08,'SCALAR');animation=dict(name='Panneau native 64 ticks',samplers=[],channels=[])
 def channel(node,path,values,kind):
  index=len(animation['samplers']);animation['samplers'].append(dict(input=times,output=acc(values,kind),interpolation='STEP'));animation['channels'].append(dict(sampler=index,target=dict(node=node,path=path)))
 for i,n in body+shadows:
  phase=n['extras'].get('native_body_frame',n['extras'].get('native_frame'));values=np.zeros((33,3),np.float32);values[phase]=1
  if phase==0:values[32]=1
  channel(i,'scale',values,'VEC3')
 angles=angle-np.arange(33)*math.pi/16;quaternions=np.zeros((33,4));quaternions[:,1]=np.sin(angles/2);quaternions[:,3]=np.cos(angles/2);channel(root[0],'rotation',quaternions,'VEC4');doc['animations']=[animation];doc.setdefault('extras',{})['nativeTiming']=dict(ticks_per_second=25,ticks_per_pose=2,pose_count=32,cycle_seconds=2.56);binary.extend(b'\0'*(-len(binary)%4));doc['buffers'][0]['byteLength']=len(binary);encoded=json.dumps(doc,separators=(',',':')).encode();encoded+=b' '*(-len(encoded)%4);bodybytes=struct.pack('<II',len(encoded),0x4e4f534a)+encoded+struct.pack('<II',len(binary),0x004e4942)+binary;output=dst/'animated-sign.glb';output.write_bytes(struct.pack('<III',0x46546c67,2,len(bodybytes)+12)+bodybytes);assert sha(source)==digest;write_json(dst/'export.json',dict(status='Standard animation export; browser appearance/playback review pending',model_sha256=digest,raw_export_sha256=sha(rawpath),glb_sha256=sha(output),timing=doc['extras']['nativeTiming'],body_phase_nodes=64,shadow_phase_nodes=32,animation_channels=len(animation['channels']),interpolation='STEP',source_geometry_and_images_unchanged_by_animation_append=True,limitations=['Isolated reusable asset proof, not library publication or full scene integration.','Browser material appearance and native phase selection remain to verify.']));print(dst)

if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
