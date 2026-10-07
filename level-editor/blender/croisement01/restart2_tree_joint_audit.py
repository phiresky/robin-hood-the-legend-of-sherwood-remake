"""Measure local bank topology and native wood visibility without changing geometry."""
import argparse,json,sys
from pathlib import Path
import bpy,bmesh
from mathutils import Vector
from mathutils.bvhtree import BVHTree
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(Path(__file__).parent))
from render_slots import acquire
from review_evidence import sha
from restart2_tree18 import SIN,COS
r=ROOT/'level-editor/work/croisement01-refinement/restart2';parser=argparse.ArgumentParser();parser.add_argument('worker',type=Path);parser.add_argument('--tree',type=int,required=True);parser.add_argument('--domain',type=Path,required=True);a=parser.parse_args(sys.argv[sys.argv.index('--')+1:]);worker=a.worker.resolve();assert not (worker/'inspection/joint-source-proof.json').exists();acquire();bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'));cfg=json.loads((worker/'workspace.json').read_text());objects=list(bpy.data.collections[cfg['collection_name']].all_objects)
def bvh(objects):
 verts=[];faces=[]
 for obj in objects:
  offset=len(verts);verts.extend(obj.matrix_world@v.co for v in obj.data.vertices);faces.extend(tuple(offset+i for i in face.vertices) for face in obj.data.polygons)
 return BVHTree.FromPolygons(verts,faces)
wood=bvh([o for o in objects if o.get('source_node')==f'scenery-tree{a.tree:02d}-wood']);terrain_nodes={'ground',f'tree{a.tree:02d}-local-soil-joint'}|{f'building-{i:03d}' for i in [*range(10),*range(76,81)]};terrain=bvh([o for o in objects if o.type=='MESH' and o.get('source_node') in terrain_nodes]);banks=[o for o in objects if o.type=='MESH' and o.get('source_node') in terrain_nodes];topology=[]
for bank in banks:
 bm=bmesh.new();bm.from_mesh(bank.data);topology.append(dict(node=bank.get('source_node'),vertices=len(bm.verts),faces=len(bm.faces),nonmanifold_edges=sum(not e.is_manifold for e in bm.edges),degenerate_faces=sum(f.calc_area()<1e-8 for f in bm.faces)));bm.free()
mask_path=a.domain.resolve();mask=np.asarray(Image.open(mask_path).convert('L'))>0;direction=Vector((0,-COS,SIN));hidden=[];missing=[]
left,top=next(row for row in json.loads((r.parent/'baseline/masks/manifest.json').read_text())['masks'] if row['index']==a.tree)['box_top_left']
for y,x in zip(*np.nonzero(mask)):
 origin=Vector((float(left+x)+.5,-(float(top+y)+.5)/SIN,0))+direction*5000
 hit,normal,index,distance=wood.ray_cast(origin,-direction,20000)
 if hit is None:missing.append([int(x),int(y)]);continue
 ground,_,_,ground_distance=terrain.ray_cast(origin,-direction,20000)
 if ground is not None and ground_distance<distance-.01:hidden.append([int(x),int(y),float(distance-ground_distance)])
report=dict(model_sha256=sha(worker/'model.blend'),source_domain_sha256=sha(mask_path),source_pixels=int(mask.sum()),wood_projection_misses=missing,wood_pixels_occluded_by_terrain=hidden,bank_topology=topology,status='Measurement only; no source ownership or geometry approval implied')
(worker/'inspection/joint-source-proof.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({**report,'wood_projection_misses':len(missing),'wood_pixels_occluded_by_terrain':len(hidden)},indent=2))
