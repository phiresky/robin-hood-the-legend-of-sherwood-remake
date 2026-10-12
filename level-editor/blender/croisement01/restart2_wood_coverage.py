"""Independent source-ray and topology census for private native wood candidates."""
import argparse,json,math,sys
from pathlib import Path
import bpy,bmesh,numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release
from review_evidence import sha
R=ROOT/'level-editor/work/croisement01-refinement/restart2';p=argparse.ArgumentParser();p.add_argument('--tree',type=int,required=True);p.add_argument('--revision',type=int);p.add_argument('--workspace',type=Path);p.add_argument('--model-name',choices=['model.blend','baseline.blend'],default='model.blend');a=p.parse_args(sys.argv[sys.argv.index('--')+1:]);assert a.revision is not None or a.workspace is not None;w=a.workspace.resolve() if a.workspace else R/f'tree{a.tree:02d}-v{a.revision}/assets/croisement01-tree-{a.tree:02d}';dest=w.parent.parent;out=w/'inspection/independent-wood-coverage-v2.json';assert not out.exists();acquire(slots=2)
try:
 model=w/a.model_name;out.parent.mkdir(exist_ok=True);before=sha(model);bpy.ops.wm.open_mainfile(filepath=str(model));targets=[o for o in bpy.context.scene.objects if o.type=='MESH' and o.get('asset_group')==f'croisement01-tree-{a.tree:02d}' and 'foliage' not in o.get('source_node','') and o.get('projection_component')!='crown'];assert targets
 verts=[];faces=[];topology=[];face_components=[];hits={}
 for o in targets:
  off=len(verts);verts.extend(o.matrix_world@v.co for v in o.data.vertices);faces.extend(tuple(off+i for i in f.vertices) for f in o.data.polygons);bm=bmesh.new();bm.from_mesh(o.data);todo=set(bm.verts);components=[];vertex_components={};object_index=len(topology)
  while todo:
   queue=[todo.pop()];n=0
   while queue:
    v=queue.pop();n+=1;vertex_components[v.index]=(object_index,len(components))
    for edge in v.link_edges:
     other=edge.other_vert(v)
     if other in todo:todo.remove(other);queue.append(other)
   components.append(n)
  face_components.extend(vertex_components[f.vertices[0]] for f in o.data.polygons)
  topology.append(dict(node=o.get('source_node'),vertices=len(bm.verts),faces=len(bm.faces),nonmanifold_edges=sum(not e.is_manifold for e in bm.edges),components=components));bm.free()
 tree=BVHTree.FromPolygons(verts,faces);native=json.loads((R.parent/'baseline/masks/manifest.json').read_text());row=next(x for x in native['masks'] if x['index']==a.tree);left,top=row['box_top_left'];mask=np.asarray(Image.open(dest/'wood-domain.png').convert('L'))>0;pad=np.pad(mask,1);interior=np.logical_and.reduce([pad[i:i+mask.shape[0],j:j+mask.shape[1]] for i in range(3) for j in range(3)]);s,c=math.sin(math.radians(35)),math.cos(math.radians(35));ray=Vector((0,-c,s));misses=[]
 for y,x in zip(*np.nonzero(mask)):
  origin=Vector((float(left+x)+.5,-(float(top+y)+.5)/s,0))+ray*5000
  hit,_,face,_=tree.ray_cast(origin,-ray,10000)
  if hit is None:misses.append([int(x),int(y)])
  else:
   key=face_components[face];hits[key]=hits.get(key,0)+1
 for i,row in enumerate(topology):row['component_source_hits']=[hits.get((i,j),0) for j in range(len(row['components']))]
 report=dict(status='MEASURED_NOT_APPROVED',model_sha256=before,domain_sha256=sha(dest/'wood-domain.png'),source_pixels=int(mask.sum()),missed_source_pixels=misses,missed_interior_pixels=[p for p in misses if interior[p[1],p[0]]],topology=topology,scope='Wood geometry source-ray coverage; color, canopy and bank contact excluded');assert sha(model)==before;out.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(dict(tree=a.tree,source=report['source_pixels'],misses=len(misses),interior=len(report['missed_interior_pixels']),topology=topology)))
finally:release()
