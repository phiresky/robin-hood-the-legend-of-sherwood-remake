"""Read native baseline projected triangle coverage without loading Blender."""
import argparse,hashlib,json,math,struct
from pathlib import Path
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/croisement01-refinement'
def main():
 p=argparse.ArgumentParser();p.add_argument('mask',type=int);p.add_argument('output',type=Path);a=p.parse_args();a.output.mkdir(exist_ok=False)
 path=BASE/'baseline/croisement01-volumes.scene.glb';b=path.read_bytes();n=struct.unpack_from('<I',b,12)[0];g=json.loads(b[20:20+n]);raw=b[28+n:]
 def access(i):
  d=g['accessors'][i];v=g['bufferViews'][d['bufferView']];t={5126:'<f4',5123:'<u2',5125:'<u4'}[d['componentType']];c={'SCALAR':1,'VEC2':2,'VEC3':3}[d['type']];z=np.dtype(t).itemsize;return np.ndarray((d['count'],c),dtype=t,buffer=raw,offset=v.get('byteOffset',0)+d.get('byteOffset',0),strides=(v.get('byteStride',c*z),z))
 manifest=json.loads((BASE/'baseline/masks/manifest.json').read_text());row=next(x for x in manifest['masks'] if x['index']==a.mask);mask=np.array(Image.open(BASE/'baseline/masks'/row['png']))>0;x0,y0=row['box_top_left'];height,width=mask.shape;inventory={r['source_node']:r for r in json.loads((BASE/'grouped-inventory/inventory.json').read_text())['objects']};rows=[]
 for node in g['nodes']:
  name=node.get('name','');name='building-000' if name=='terrace-000' else name
  if not name.startswith('building-'):continue
  pixels=np.zeros(mask.shape,dtype=bool);allxy=[]
  for primitive in g['meshes'][node['mesh']]['primitives']:
   pos=access(primitive['attributes']['POSITION']).astype(np.float64);xy=np.column_stack([pos[:,0],-pos[:,1]*math.sin(math.radians(35))-pos[:,2]*math.cos(math.radians(35))]);allxy.extend(xy);xy=xy-[x0,y0]
   for tri in access(primitive['indices']).reshape(-1,3):
    pts=xy[tri];lo=np.maximum(np.floor(pts.min(0)).astype(int),[0,0]);hi=np.minimum(np.ceil(pts.max(0)).astype(int),[width,height])
    if np.any(hi<=lo):continue
    yy,xx=np.mgrid[lo[1]:hi[1],lo[0]:hi[0]];q=np.stack([xx+.5,yy+.5],axis=-1);edges=[]
    for j in range(3):
     u,v=pts[j],pts[(j+1)%3];edges.append((q[...,0]-u[0])*(v[1]-u[1])-(q[...,1]-u[1])*(v[0]-u[0]))
    edges=np.stack(edges);pixels[lo[1]:hi[1],lo[0]:hi[0]]|=np.all(edges>=-1e-8,axis=0)|np.all(edges<=1e-8,axis=0)
  pts=np.array(allxy);bounds=[float(v) for v in [*pts.min(0),*pts.max(0)]];expected=inventory[name]['bounds_source_pixels'];assert np.max(np.abs(np.array(bounds)-expected))<.001,(name,bounds,expected)
  count=int((pixels&mask).sum())
  if count:
   image=a.output/(name+'-overlap.png');Image.fromarray(((pixels&mask)*255).astype('uint8')).save(image);rows.append(dict(node=name,overlap_pixels=count,bounds_source=bounds,domain_sha256=hashlib.sha256(image.read_bytes()).hexdigest()))
 d=dict(status='Projected triangle coverage only; visual frontmost ownership requires later depth proof',native_mask=a.mask,source_glb_sha256=hashlib.sha256(b).hexdigest(),projected_bounds_match_independent_inventory=True,rows=rows);(a.output/'report.json').write_text(json.dumps(d,indent=2)+'\n');print(json.dumps(d,indent=2))
if __name__=='__main__':main()
