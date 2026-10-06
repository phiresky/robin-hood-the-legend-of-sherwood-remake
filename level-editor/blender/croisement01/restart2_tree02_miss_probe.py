"""Classify exact source-domain misses with per-pixel subpixel ray evidence."""
import json,math,sys
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
from review_evidence import sha
R=ROOT/'level-editor/work/croisement01-refinement/restart2';revision=int(sys.argv[sys.argv.index('--')+1]) if '--' in sys.argv else 6;v=R/f'tree02-v{revision}';w=v/'assets/croisement01-tree-02';out=v/'miss-classification-v1.json';assert not out.exists();acquire();bpy.ops.wm.open_mainfile(filepath=str(w/'model.blend'));cfg=json.loads((w/'workspace.json').read_text());verts=[];faces=[]
for o in bpy.data.collections[cfg['collection_name']].all_objects:
 if o.type!='MESH' or o.get('asset_group')!=cfg['asset_id']:continue
 offset=len(verts);verts.extend(o.matrix_world@x.co for x in o.data.vertices);faces.extend(tuple(offset+i for i in f.vertices) for f in o.data.polygons)
bvh=BVHTree.FromPolygons(verts,faces);domain=np.asarray(Image.open(v/'wood-domain.png'))>0;colors=np.asarray(Image.open(w/'inspection/native-geometry-coverage/comparison.png'))[::4,::4,:3];missing=(colors==[255,50,50]).all(2);expected_misses=json.loads((w/'inspection/native-geometry-coverage/report.json').read_text())['missing_pixels'];assert int(missing.sum())==expected_misses;pad=np.pad(domain,1);interior=np.ones_like(domain)
for dy in [-1,0,1]:
 for dx in [-1,0,1]:interior&=pad[1+dy:1+dy+domain.shape[0],1+dx:1+dx+domain.shape[1]]
s,c=math.sin(math.radians(35)),math.cos(math.radians(35));away=Vector((0,-c,s));records=[]
for y,x in zip(*np.where(missing)):
 hits=[]
 for dy in [-.4,-.2,0,.2,.4]:
  for dx in [-.4,-.2,0,.2,.4]:
   target=Vector((101+x+.5+dx,-(y+.5+dy)/s,0));hits.append(bvh.ray_cast(target+away*5000,-away,10000)[0] is not None)
 nearest=None
 for radius in [.5,.75,1,1.25,1.5]:
  found=False
  for angle in np.linspace(0,math.tau,24,endpoint=False):
   target=Vector((101+x+.5+math.cos(angle)*radius,-(y+.5+math.sin(angle)*radius)/s,0));found|=bvh.ray_cast(target+away*5000,-away,10000)[0] is not None
  if found:nearest=radius;break
 records.append(dict(nearest_ring_hit_distance=nearest,source_pixel=[int(x+101),int(y)],domain_interior_8neighbors=bool(interior[y,x]),subpixel_hits=sum(hits),subpixel_samples=25))
summary=dict(total=len(records),interior=sum(r['domain_interior_8neighbors'] for r in records),boundary=sum(not r['domain_interior_8neighbors'] for r in records),partial_sample_hit=sum(r['subpixel_hits']>0 for r in records),no_subpixel_hit=sum(r['subpixel_hits']==0 for r in records))
out.write_text(json.dumps(dict(model_sha256=sha(w/'model.blend'),mask_sha256=sha(v/'wood-domain.png'),summary=summary,pixels=records,scope='All recorded center-ray misses classified; partial hit is not a claim that painted source is fully preserved.'),indent=2)+'\n');print(json.dumps(summary))
