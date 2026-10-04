"""Verify standard sign animation timing, geometry and packed source appearance."""
import json,sys,struct,io,math
from pathlib import Path
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json

def read(path):
 raw=path.read_bytes();size=struct.unpack_from('<I',raw,12)[0];return json.loads(raw[20:20+size]),raw[28+size:]

def main():
 base=OUT/'state-sign-candidate';dst=base/'animation-export-v2';path=dst/'animated-sign.glb';doc,binary=read(path);source,original=read(dst/'static-source.glb');assert doc['meshes']==source['meshes'];assert binary[:len(original)]==original
 def view(index):
  v=doc['bufferViews'][index];o=v.get('byteOffset',0);return binary[o:o+v['byteLength']]
 def acc(index):
  a=doc['accessors'][index];assert a['componentType']==5126;return np.frombuffer(view(a['bufferView']),np.float32,offset=a.get('byteOffset',0),count=a['count']*{'SCALAR':1,'VEC3':3,'VEC4':4}[a['type']]).reshape(a['count'],-1)
 animation=doc['animations'][0];assert len(doc['animations'])==1 and len(animation['channels'])==97
 rotations=0;scales=0
 for c in animation['channels']:
  sampler=animation['samplers'][c['sampler']];assert sampler['interpolation']=='STEP';times=acc(sampler['input']).ravel();assert np.allclose(times,np.arange(33)*.08,atol=1e-7);values=acc(sampler['output']);node=doc['nodes'][c['target']['node']]
  if c['target']['path']=='scale':
   phase=node['extras'].get('native_body_frame',node['extras'].get('native_frame'));expected=np.zeros((33,3));expected[phase]=1
   if phase==0:expected[32]=1
   assert np.array_equal(values,expected);scales+=1
  else:
   assert c['target']['path']=='rotation' and node['name']=='Rotating sign pivot';angle=2*math.atan2(node['rotation'][1],node['rotation'][3]);expected=np.zeros((33,4));angles=angle-np.arange(33)*math.pi/16;expected[:,1]=np.sin(angles/2);expected[:,3]=np.cos(angles/2);assert np.allclose(values,expected,atol=1e-6);rotations+=1
 assert scales==96 and rotations==1
 body_images=0;shadow_images=0
 for material in doc['materials']:
  name=material['name']
  if name.startswith('Native sign pose '):
   fields=name.split();phase=int(fields[3]);part=fields[4];face=int(fields[6]);texture=material['emissiveTexture']['index'];expected=np.asarray(Image.open(base/f'phase-appearance-v1/pose-{phase:02}-{part}-face-{face}.png').convert('RGB'));image=doc['images'][doc['textures'][texture]['source']];actual=np.asarray(Image.open(io.BytesIO(view(image['bufferView']))).convert('RGB'));assert np.array_equal(actual,expected);body_images+=1
  elif name.startswith('Native black ground stroke '):
   phase=int(name.rsplit(' ',1)[-1]);assert material['alphaMode']=='MASK' and material['alphaCutoff']==.5 and 'KHR_materials_unlit'in material['extensions'];texture=material['pbrMetallicRoughness']['baseColorTexture']['index'];image=doc['images'][doc['textures'][texture]['source']];assert view(image['bufferView'])==(base/f'painted-shadow-v2/shadow-{phase:02}.png').read_bytes();shadow_images+=1
 assert body_images==384 and shadow_images==32;browser=json.loads((dst/'browser-verification.json').read_text());assert browser['status']=='PASS' and len(browser['samples'])==32;write_json(dst/'verification.json',dict(status='PASS isolated export geometry, packed RGB/alpha, native timing and browser node playback',glb_sha256=sha(path),model_sha256=json.loads((dst/'export.json').read_text())['model_sha256'],browser_verification_sha256=sha(dst/'browser-verification.json'),unchanged_geometry=True,known_image_rgb_changed=0,body_face_images=body_images,exact_shadow_rgba_images=shadow_images,scale_channels=scales,rotation_channels=rotations,cycle_seconds=2.56,limitations=['Independent coordinator review and complete physical scene/library integration remain open.','Browser screenshots require visual inspection in addition to these structural checks.']));print('Verified384 body RGB images,32 exact shadow RGBA masks and97 native-timed STEP channels')

if __name__=='__main__':main()
