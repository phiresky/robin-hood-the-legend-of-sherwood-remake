"""Read-only native baseline visibility across a privately changed terrain region."""
import argparse,json,math,struct,sys
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
from review_evidence import sha
p=argparse.ArgumentParser();p.add_argument('worker',type=Path);p.add_argument('--mask',type=int,default=8);p.add_argument('--nodes',nargs='+',default=['building-033']);a=p.parse_args(sys.argv[sys.argv.index('--')+1:]);w=a.worker.resolve();out=w/'inspection'/f'baseline-mask{a.mask:02d}-visibility.json';assert not out.exists();base=ROOT/'level-editor/work/croisement01-refinement';glb=base/'baseline/croisement01-volumes.scene.glb';raw=glb.read_bytes();n=struct.unpack_from('<I',raw,12)[0];g=json.loads(raw[20:20+n]);binary=raw[28+n:]
def access(i):
 d=g['accessors'][i];v=g['bufferViews'][d['bufferView']];dtype={5126:'<f4',5123:'<u2',5125:'<u4'}[d['componentType']];width={'SCALAR':1,'VEC2':2,'VEC3':3}[d['type']];size=np.dtype(dtype).itemsize;return np.ndarray((d['count'],width),dtype=dtype,buffer=binary,offset=v.get('byteOffset',0)+d.get('byteOffset',0),strides=(v.get('byteStride',width*size),size))
inventory={r['source_node']:r for r in json.loads((base/'grouped-inventory/inventory.json').read_text())['objects']};s,c=math.sin(math.radians(35)),math.cos(math.radians(35));direction=Vector((0,-c,s));meshes={}
for node in g['nodes']:
 name=node.get('name');name='building-000' if name=='terrace-000' else name
 if name not in set(a.nodes)|{'building-000','building-006','building-007'}:continue
 points=[];faces=[]
 for primitive in g['meshes'][node['mesh']]['primitives']:
  offset=len(points);points.extend(Vector(row) for row in access(primitive['attributes']['POSITION']));faces.extend(tuple(offset+int(k) for k in tri) for tri in access(primitive['indices']).reshape(-1,3))
 xy=np.asarray([[v.x,-v.y*s-v.z*c] for v in points]);bounds=[*xy.min(0),*xy.max(0)];assert max(abs(x-y) for x,y in zip(bounds,inventory[name]['bounds_source_pixels']))<.001
 meshes[name]=(points,faces)
def combine(parts):
 points=[];faces=[]
 for pp,ff in parts:
  offset=len(points);points.extend(pp);faces.extend(tuple(offset+k for k in f) for f in ff)
 return BVHTree.FromPolygons(points,faces)
old=combine([meshes[n] for n in ['building-000','building-006','building-007']]);acquire();bpy.ops.wm.open_mainfile(filepath=str(w/'model.blend'));cfg=json.loads((w/'workspace.json').read_text());parts=[]
for o in bpy.data.collections[cfg['collection_name']].all_objects:
 if o.type=='MESH' and o.get('source_node') in {'building-000','building-006','building-007'}:parts.append(([o.matrix_world@v.co for v in o.data.vertices],[tuple(f.vertices) for f in o.data.polygons]))
new=combine(parts);row=next(row for row in json.loads((base/'baseline/masks/manifest.json').read_text())['masks'] if row['index']==a.mask);mask=Image.open(base/'baseline/masks'/row['png']).convert('L');left,top=row['box_top_left'];records=[]
for name in a.nodes:
 wood=combine([meshes[name]]);hits=0;old_hidden=[];new_hidden=[];newly_hidden=[]
 for y,x in zip(*np.nonzero(np.asarray(mask))):
  origin=Vector((float(left+x)+.5,-(float(top+y)+.5)/s,0))+direction*5000;hit,_,_,d=wood.ray_cast(origin,-direction,20000)
  if hit is None:continue
  hits+=1;old_hit,_,_,old_d=old.ray_cast(origin,-direction,20000);new_hit,_,_,new_d=new.ray_cast(origin,-direction,20000);was_hidden=old_hit is not None and old_d<d-.01;is_hidden=new_hit is not None and new_d<d-.01
  if was_hidden:old_hidden.append([int(x),int(y)])
  if is_hidden:new_hidden.append([int(x),int(y)])
  if is_hidden and not was_hidden:newly_hidden.append([int(x),int(y),float(d-new_d)])
 records.append(dict(node=name,native_ray_hits=hits,already_hidden_by_original_terrain=old_hidden,hidden_by_candidate=new_hidden,newly_hidden=newly_hidden))
report=dict(status='Baseline visibility measurement, not semantic assignment or approval',candidate_sha256=sha(w/'model.blend'),baseline_glb_sha256=sha(glb),mask=a.mask,mask_sha256=sha(base/'baseline/masks'/row['png']),baseline_bounds_independently_verified=True,records=records);out.write_text(json.dumps(report,indent=2)+'\n');print([(r['node'],r['native_ray_hits'],len(r['already_hidden_by_original_terrain']),len(r['newly_hidden'])) for r in records])
