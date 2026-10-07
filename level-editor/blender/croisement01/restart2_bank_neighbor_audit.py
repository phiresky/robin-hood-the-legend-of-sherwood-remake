"""Compare changed banks against frozen neighboring woody source rays."""
import argparse,json,math,sys
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
from review_evidence import sha
p=argparse.ArgumentParser();p.add_argument('worker',type=Path);a=p.parse_args(sys.argv[sys.argv.index('--')+1:]);w=a.worker.resolve();out=w/'inspection/neighbor-source-proof-v2.json';assert not out.exists();R=ROOT/'level-editor/work/croisement01-refinement/restart2';construction=json.loads((w.parents[1]/'construction.json').read_text());acquire();bpy.ops.wm.open_mainfile(filepath=str(w/'model.blend'));cfg=json.loads((w/'workspace.json').read_text());objects=list(bpy.data.collections[cfg['collection_name']].all_objects)
def bvh(objects):
 points=[];faces=[]
 for o in objects:
  off=len(points);points.extend(o.matrix_world@v.co for v in o.data.vertices);faces.extend(tuple(off+k for k in f.vertices) for f in o.data.polygons)
 return BVHTree.FromPolygons(points,faces)
banks=bvh([o for o in objects if o.type=='MESH' and o.get('source_node') in construction['changed_nodes']]);native=json.loads((R.parent/'baseline/masks/manifest.json').read_text());sine,cosine=math.sin(math.radians(35)),math.cos(math.radians(35));direction=Vector((0,-cosine,sine));records=[]
neighbors=[(0,'approved-tree00-wood-fill-v1/croisement01-tree-00/baked-v4-support/worker.blend','tree00-v4/wood-domain.png'),(1,'approved-tree01-isolated-wood-fill-v1/croisement01-tree-01/baked-v1-luminance/worker.blend','tree01-source-prep-v1/wood-domain-proposal.png'),(2,'tree02-v8/assets/croisement01-tree-02/model.blend','tree02-v8/wood-domain.png'),(3,'approved-tree03-fill-v1/croisement01-tree-03/baked-v1-luminance/worker.blend','tree03-v4/wood-domain.png')]
if any(row['mask']==6 for row in construction['root_constraints']):neighbors.append((6,'tree06-v6/assets/croisement01-tree-06/model.blend','tree06-v6/wood-domain.png'))
for n,path,mask_path in neighbors:
 path=R/path;mask_path=R/mask_path
 with bpy.data.libraries.load(str(path),link=False) as (src,dst):dst.objects=list(src.objects)
 loaded=[o for o in dst.objects if o is not None]
 for imported in loaded:bpy.context.scene.collection.objects.link(imported)
 bpy.context.view_layer.update()
 targets=[o for o in loaded if o.type=='MESH' and o.get('asset_group')==f'croisement01-tree-{n:02d}' and 'foliage' not in o.get('source_node','') and o.get('projection_component')!='crown'];assert targets
 reference=next(row for row in json.loads((R/('bank-neighbor-transform-reference-v2.json' if n==6 else 'bank-neighbor-transform-reference-v1.json')).read_text())['sources'] if row['mask']==n);assert reference['model_sha256']==sha(path)
 assert len(reference['objects'])==len(targets)
 for target in targets:
  expected=[row for row in reference['objects'] if row['source_node']==target.get('source_node')];assert len(expected)==1
  assert max(abs(target.matrix_world[i][j]-expected[0]['matrix_world'][i][j]) for i in range(4) for j in range(4))<1e-5,'Neighbor evaluated transform mismatch'
 wood=bvh(targets);mask=np.asarray(Image.open(mask_path).convert('L'))>0;left,top=next(r for r in native['masks'] if r['index']==n)['box_top_left'];hits=0;hidden=[]
 for y,x in zip(*np.nonzero(mask)):
  origin=Vector((float(left+x)+.5,-(float(top+y)+.5)/sine,0))+direction*5000;hit,_,_,distance=wood.ray_cast(origin,-direction,20000)
  if hit is None:continue
  hits+=1;ground,_,_,ground_distance=banks.ray_cast(origin,-direction,20000)
  if ground is not None and ground_distance<distance-.01:hidden.append([int(x),int(y),float(distance-ground_distance)])
 records.append(dict(mask=n,model_sha256=sha(path),domain_sha256=sha(mask_path),wood_ray_hits=hits,pixels_occluded_by_changed_banks=hidden))
 for o in loaded:bpy.data.objects.remove(o,do_unlink=True)
out.write_text(json.dumps(dict(status='Independent changed-bank occlusion measurement; no neighbor appearance or geometry approval',candidate_sha256=sha(w/'model.blend'),changed_nodes=construction['changed_nodes'],neighbors=records),indent=2)+'\n');print([(r['mask'],len(r['pixels_occluded_by_changed_banks'])) for r in records])
