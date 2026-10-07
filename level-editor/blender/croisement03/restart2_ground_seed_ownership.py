"""Reconstruct frozen coarse foreground coverage; distinguish old fill from ground."""
import hashlib,json,math,struct
from pathlib import Path
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];B=ROOT/'level-editor/work/croisement03-refinement/restart2';OUT=B/'trio-tree-integration-v1/seed-ground-ownership-v2'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 OUT.mkdir(exist_ok=False);p=B.parent/'baseline/croisement03-volumes.scene.glb';data=p.read_bytes();n=struct.unpack_from('<I',data,12)[0];g=json.loads(data[20:20+n]);binary=data[28+n:]
 def acc(i):
  a=g['accessors'][i];v=g['bufferViews'][a['bufferView']];dt={5126:'<f4',5123:'<u2',5125:'<u4'}[a['componentType']];size={'SCALAR':1,'VEC2':2,'VEC3':3,'VEC4':4}[a['type']];stride=v.get('byteStride',np.dtype(dt).itemsize*size);return np.ndarray((a['count'],size),dtype=dt,buffer=binary,offset=v.get('byteOffset',0)+a.get('byteOffset',0),strides=(stride,np.dtype(dt).itemsize)).copy()
 w,h=1408,960;depth=np.full((h,w),np.inf,np.float32);owner=np.full((h,w),-1,np.int32);sin,cos=math.sin(math.radians(35)),math.cos(math.radians(35));forward=np.array([0,cos,-sin]);count=0
 for node in g['nodes']:
  if not node.get('name','').startswith('building-'):continue
  assert all(k not in node for k in ('matrix','translation','rotation','scale'));identity=int(node['name'].split('-')[-1]);count+=1
  for prim in g['meshes'][node['mesh']]['primitives']:
   pos=acc(prim['attributes']['POSITION']).astype(float);indices=acc(prim['indices']).reshape(-1,3);xy=np.column_stack((pos[:,0],-pos[:,1]*sin-pos[:,2]*cos)).astype(np.float32);pd=(pos@forward).astype(np.float32)
   for ids in indices:
    pa,pb,pc=pos[ids]
    if np.cross(pb-pa,pc-pa)@forward>=0:continue
    a,b,c=xy[ids].astype(float);lo=np.maximum(np.floor(np.minimum(np.minimum(a,b),c)).astype(int),0);hi=np.minimum(np.ceil(np.maximum(np.maximum(a,b),c)).astype(int),[w-1,h-1]);area=(b[0]-a[0])*(c[1]-a[1])-(b[1]-a[1])*(c[0]-a[0])
    if abs(area)<1e-9 or np.any(lo>hi):continue
    xx,yy=np.meshgrid(np.arange(lo[0],hi[0]+1)+.5,np.arange(lo[1],hi[1]+1)+.5);l0=((b[0]-xx)*(c[1]-yy)-(c[0]-xx)*(b[1]-yy))/area;l1=((c[0]-xx)*(a[1]-yy)-(a[0]-xx)*(c[1]-yy))/area;l2=1-l0-l1;dist=l0*pd[ids[0]]+l1*pd[ids[1]]+l2*pd[ids[2]];region=depth[lo[1]:hi[1]+1,lo[0]:hi[0]+1];hit=(l0>=0)&(l1>=0)&(l2>=0)&(dist<region);region[hit]=dist[hit];owner[lo[1]:hi[1]+1,lo[0]:hi[0]+1][hit]=identity
 assert count==106,count;np.savez_compressed(OUT/'coarse-owners.npz',owner=owner);Image.fromarray((owner>=0).astype('uint8')*255).save(OUT/'original-coarse-fill-domain.png');candidate=np.array(Image.open(B/'trio-tree-integration-v1/terrain-input-proposal-v1/candidate-unknown-domain.png'))>0;rows=[]
 for i in np.unique(owner[candidate]):rows.append(dict(seed_owner=int(i),pixels=int((candidate&(owner==i)).sum())))
 receipt=dict(status='READ-ONLY prior coarse source-role classification, not current foliage ownership',frozen_seed_sha256=sha(p),seed_obstacles=count,algorithm_reference='level-editor/pipeline/src/volume-raster.ts',algorithm_sha256=sha(ROOT/'level-editor/pipeline/src/volume-raster.ts'),ground_fill_rule_reference='level-editor/pipeline/src/volume-fill.ts:1431',candidate_pixels=int(candidate.sum()),candidate_inside_prior_coarse_fill=int((candidate&(owner>=0)).sum()),candidate_outside_prior_coarse_fill=int((candidate&(owner<0)).sum()),seed_owner_breakdown=rows,limits=['Positive coarse owner means the old ground recipe treated that pixel as unknown beneath a volume and synthesized it. It does not assign present foliage ownership.','Negative owner means retained native map-art RGB in old ground; it can still depict unmodeled foreground foliage rather than real floor.','Projection uses frozen exported float32 seed triangles and the existing front-face pixel-center raster rule; no model or image modified.','Image JPEG compression is separate from source-role identity.']);(OUT/'receipt.json').write_text(json.dumps(receipt,indent=2)+'\n');print(json.dumps(receipt,indent=2))
if __name__=='__main__':main()
