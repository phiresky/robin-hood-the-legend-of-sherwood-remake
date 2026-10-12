"""Measure baseline projected proxy depth; never infer gameplay ownership from it."""
import argparse,hashlib,json,math,struct
from pathlib import Path
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/active/croisement01'
p=argparse.ArgumentParser();p.add_argument('mask',type=int);p.add_argument('output',type=Path);p.add_argument('--domain',type=Path);a=p.parse_args();a.output.mkdir(exist_ok=False)
b=(BASE/'baseline/croisement01-volumes.scene.glb').read_bytes();n=struct.unpack_from('<I',b,12)[0];g=json.loads(b[20:20+n]);raw=b[28+n:]
def access(i):
 d=g['accessors'][i];v=g['bufferViews'][d['bufferView']];t={5126:'<f4',5123:'<u2',5125:'<u4'}[d['componentType']];c={'SCALAR':1,'VEC2':2,'VEC3':3}[d['type']];z=np.dtype(t).itemsize;return np.ndarray((d['count'],c),dtype=t,buffer=raw,offset=v.get('byteOffset',0)+d.get('byteOffset',0),strides=(v.get('byteStride',c*z),z))
manifest=json.loads((BASE/'baseline/masks/manifest.json').read_text());row=next(x for x in manifest['masks'] if x['index']==a.mask);maskpath=a.domain or BASE/'baseline/masks'/row['png'];mask=np.asarray(Image.open(maskpath).convert('L'))>0;x0,y0=row['box_top_left'];height,width=mask.shape;depth=np.full(mask.shape,-np.inf);owner=np.full(mask.shape,-1,dtype=int);inventory={r['source_node']:r for r in json.loads((BASE/'grouped-inventory/inventory.json').read_text())['objects']};sn,cs=math.sin(math.radians(35)),math.cos(math.radians(35));names=[]
for node in g['nodes']:
 name=node.get('name','');name='building-000' if name=='terrace-000' else name
 if not name.startswith('building-'):continue
 node_index=len(names);names.append(name);allxy=[]
 for primitive in g['meshes'][node['mesh']]['primitives']:
  pos=access(primitive['attributes']['POSITION']).astype(float);xy=np.column_stack([pos[:,0],-pos[:,1]*sn-pos[:,2]*cs]);allxy.extend(xy);xy=xy-np.array([x0,y0]);z=-pos[:,1]*cs+pos[:,2]*sn
  for indices in access(primitive['indices']).reshape(-1,3):
   pts=xy[indices];lo=np.maximum(np.floor(pts.min(0)).astype(int),[0,0]);hi=np.minimum(np.ceil(pts.max(0)).astype(int),[width,height])
   if np.any(hi<=lo):continue
   pa,pb,pc=pts;den=(pb[1]-pc[1])*(pa[0]-pc[0])+(pc[0]-pb[0])*(pa[1]-pc[1])
   if abs(den)<1e-8:continue
   yy,xx=np.mgrid[lo[1]:hi[1],lo[0]:hi[0]];xx=xx+.5;yy=yy+.5;wa=((pb[1]-pc[1])*(xx-pc[0])+(pc[0]-pb[0])*(yy-pc[1]))/den;wb=((pc[1]-pa[1])*(xx-pc[0])+(pa[0]-pc[0])*(yy-pc[1]))/den;wc=1-wa-wb;inside=(wa>=-1e-8)&(wb>=-1e-8)&(wc>=-1e-8);current=depth[lo[1]:hi[1],lo[0]:hi[0]];candidate=wa*z[indices[0]]+wb*z[indices[1]]+wc*z[indices[2]];take=inside&(candidate>current);current[take]=candidate[take];owner[lo[1]:hi[1],lo[0]:hi[0]][take]=node_index
 points=np.asarray(allxy);bounds=np.r_[points.min(0),points.max(0)];assert np.max(np.abs(bounds-inventory[name]['bounds_source_pixels']))<.001
rows=[dict(node=name,frontmost_proxy_pixels=int(np.count_nonzero(mask&(owner==i)))) for i,name in enumerate(names) if np.any(mask&(owner==i))];rows.sort(key=lambda x:-x['frontmost_proxy_pixels']);Image.fromarray(np.where(mask,owner+1,0).astype(np.uint16)).save(a.output/'proxy-owner-indices.png');report=dict(status='GEOMETRIC_PROXY_DEPTH_DIAGNOSTIC_NOT_SEMANTIC_OR_GAMEPLAY_ASSOCIATION',native_mask=a.mask,domain_sha256=hashlib.sha256(maskpath.read_bytes()).hexdigest(),baseline_glb_sha256=hashlib.sha256(b).hexdigest(),pixels=int(mask.sum()),uncovered=int(np.count_nonzero(mask&(owner<0))),rows=rows,index_names=names,limitations='Proxy solid geometry ignores material cutouts and does not establish native mask-obstacle links. Source art and logical scope need independent review.');(a.output/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(rows))
