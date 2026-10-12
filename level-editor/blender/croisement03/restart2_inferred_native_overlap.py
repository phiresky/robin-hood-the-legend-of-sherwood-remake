"""Conservative CPU native-plane occlusion probe; not a rendered appearance proof."""
import io,json,math
from pathlib import Path
import numpy as np
from PIL import Image
from restart2_static_pair_export_guard import read_glb
ROOT=Path(__file__).resolve().parents[3]
BASE=ROOT/'level-editor/work/active/croisement03/restart2'
OUT=BASE/'combined-trio-inferred-crown-candidate-v1'
s,c=math.sin(math.radians(35)),math.cos(math.radians(35))

def transform(node):
 if 'matrix' in node:return np.array(node['matrix']).reshape(4,4).T
 x,y,z,w=node.get('rotation',[0,0,0,1]);q=np.array([[1-2*(y*y+z*z),2*(x*y-z*w),2*(x*z+y*w)],[2*(x*y+z*w),1-2*(x*x+z*z),2*(y*z-x*w)],[2*(x*z-y*w),2*(y*z+x*w),1-2*(x*x+y*y)]])
 m=np.eye(4);m[:3,:3]=q@np.diag(node.get('scale',[1,1,1]));m[:3,3]=node.get('translation',[0,0,0]);return m

def main():
 destination=OUT/'native-overlap-conservative-v3.json';assert not destination.exists()
 contract=json.loads((BASE/'combined-trio-stage-v2/animation-contract.json').read_text());union=np.zeros((1160,1408),bool)
 for frame in contract['frameProof']:
  rgba=np.array(Image.open(BASE/'combined-trio-stage-v2/library'/frame['path']).convert('RGBA'));x=665+frame['offset'][0];y=frame['offset'][1]+200;h,w=rgba.shape[:2];union[y:y+h,x:x+w]|=rgba[:,:,3]>0
 rows=[]
 for number in (12,14):
  doc,blob=read_glb(OUT/f'tree{number}/model.glb');desc=json.loads((OUT/f'tree{number}/asset.json').read_text());origin=np.array(desc['source_origin_scene']);hit=np.zeros_like(union);coverage=np.zeros_like(union);diagnostics=[]
  def read(index):
   a=doc['accessors'][index];v=doc['bufferViews'][a['bufferView']];dtype=np.dtype({5126:'<f4',5125:'<u4',5123:'<u2',5121:'u1'}[a['componentType']]);n={'SCALAR':1,'VEC2':2,'VEC3':3,'VEC4':4}[a['type']];return np.ndarray((a['count'],n),dtype=dtype,buffer=blob,offset=v.get('byteOffset',0)+a.get('byteOffset',0),strides=(v.get('byteStride',dtype.itemsize*n),dtype.itemsize)).copy()
  def visit(index,parent):
   node=doc['nodes'][index];world=parent@transform(node)
   if 'mesh' in node:
    for p in doc['meshes'][node['mesh']]['primitives']:
     mat=doc['materials'][p['material']]
     if mat.get('extras',{}).get('crown_source_role')!='inferred-crown-context':continue
     pos=read(p['attributes']['POSITION']);g=(world@np.c_[pos,np.ones(len(pos))].T).T[:,:3];scene=np.c_[g[:,0],-g[:,2],g[:,1]]+origin;screen=np.c_[scene[:,0],-scene[:,1]*s-scene[:,2]*c+200];uv=read(p['attributes']['TEXCOORD_0']);texture=doc['textures'][mat['pbrMetallicRoughness']['baseColorTexture']['index']];im=doc['images'][texture['source']];v=doc['bufferViews'][im['bufferView']];start=v.get('byteOffset',0);rgba=np.array(Image.open(io.BytesIO(blob[start:start+v['byteLength']])).convert('RGBA'));th,tw=rgba.shape[:2];diagnostics.append({'screen_bounds':[screen.min(0).tolist(),screen.max(0).tolist()],'alpha_values':np.unique(rgba[:,:,3]).tolist(),'uv_bounds':[uv.min(0).tolist(),uv.max(0).tolist()]})
     for ids in read(p['indices']).ravel().reshape(-1,3):
      tri=screen[ids];lo=np.maximum(np.floor(tri.min(0)).astype(int),[0,0]);hi=np.minimum(np.ceil(tri.max(0)).astype(int),[1407,1159])
      if np.any(hi<lo):continue
      x,y=np.meshgrid(np.arange(lo[0],hi[0]+1)+.5,np.arange(lo[1],hi[1]+1)+.5);a,b,d=tri;den=(b[1]-d[1])*(a[0]-d[0])+(d[0]-b[0])*(a[1]-d[1])
      if abs(den)<1e-10:continue
      u=((b[1]-d[1])*(x-d[0])+(d[0]-b[0])*(y-d[1]))/den;v=((d[1]-a[1])*(x-d[0])+(a[0]-d[0])*(y-d[1]))/den;weights=np.stack([u,v,1-u-v],-1);inside=np.all(weights>=-1e-7,axis=-1)
      coords=weights@uv[ids];tx=np.clip(np.floor(coords[:,:,0]*tw).astype(int),0,tw-1);ty=np.clip(np.floor(coords[:,:,1]*th).astype(int),0,th-1);opaque=inside&(rgba[ty,tx,3]>=128);world_y=weights@scene[ids,1];region=np.s_[lo[1]:hi[1]+1,lo[0]:hi[0]+1];coverage[region]|=opaque;hit[region]|=opaque&(world_y < -200/s-1e-4)&union[region]
   for child in node.get('children',[]):visit(child,world)
  for index in doc['scenes'][doc.get('scene',0)]['nodes']:visit(index,np.eye(4))
  rows.append({'tree':number,'diagnostics':diagnostics,'inferred_opaque_projected_pixels':int(coverage.sum()),'in_map_opaque_pixels':int(coverage[200:].sum()),'above_map_opaque_pixels':int(coverage[:200].sum()),'potential_native_front_occlusion_pixels':int(hit.sum()),'first_examples_xy':(np.argwhere(hit)[:12,::-1]-np.array([0,200])).tolist()})
 result={'status':'HOLD_CONSERVATIVE_SOURCE_OVERLAP' if any(r['potential_native_front_occlusion_pixels']for r in rows)else'HOLD_DIAGNOSTIC_ZERO_COVERAGE' if any(not r['inferred_opaque_projected_pixels'] for r in rows) else 'NO_CONSERVATIVE_NATIVE_FRONT_OVERLAP','rows':rows,'native_union_pixels':int(union.sum()),'diagnostic_coordinate_note':'Screen bounds use y+200 to retain out-of-map canopy evidence. Earlier v1/v2 probes omitted above-map pixels; v1 zero is not a source guard PASS.','method':'Exact candidate indexed faces, accumulated glTF transforms and descriptor origin; native camera pixel centers; physical alpha nearest sampling; front of native plane y=-200/sin35; union of raw-alpha14nativeframes.','limitations':['Raw alpha union is conservative; runtime checkerboard decoding may remove pixels. Positive counts require exact runtime first-hit proof, not automatic geometry deletion.','Static, bark and fern first-hit guards remain pending. No render or appearance pass inferred.']}
 destination.write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result))
if __name__=='__main__':main()
