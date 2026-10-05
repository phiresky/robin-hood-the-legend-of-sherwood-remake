"""Measure nearby bank surfaces capable of supporting the observed tree root."""
import json
import math
import sys
from pathlib import Path
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image
import numpy as np
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
from review_evidence import sha
OUT=ROOT/'level-editor/work/croisement01-refinement'
R=OUT/'restart2';w=R/'tree21-v5/assets/croisement01-tree-21'
acquire();bpy.ops.wm.open_mainfile(filepath=str(w/'model.blend'))
working=bpy.data.collections['Croisement01 Working'];s,c=math.sin(math.radians(35)),math.cos(math.radians(35))
obj=next(o for o in working.all_objects if o.type=='MESH' and o.get('source_node')=='building-073')
vertices=[];faces=[];nodes={'ground'}|{f'building-{i:03}' for i in [*range(10),*range(76,81)]}
for o in working.all_objects:
 if o.type!='MESH' or o.get('source_node') not in nodes:continue
 n=len(vertices);vertices.extend(o.matrix_world@v.co for v in o.data.vertices);faces.extend(tuple(n+i for i in f.vertices) for f in o.data.polygons)
terrain=BVHTree.FromPolygons(vertices,faces)
points=[obj.matrix_world@v.co for v in obj.data.vertices];zmin=min(p.z for p in points)
root=[p for p in points if p.z<zmin+4];center=sum(root,Vector())/len(root)
alpha=np.asarray(Image.open(OUT/'baseline/masks/000021.png').convert('L'))>127
records=[]
for x in range(1255,1301,3):
 for dy in range(-45,46,3):
  y=center.y+dy;hit,normal,face,distance=terrain.ray_cast(Vector((x,y,center.z+80)),Vector((0,0,-1)),160)
  if hit is None:continue
  py=-hit.y*s-hit.z*c;ix=int(math.floor(x))-1256;iy=int(math.floor(py))
  inside=0<=iy<alpha.shape[0] and 0<=ix<alpha.shape[1] and bool(alpha[iy,ix])
  if inside and iy>=210:
   records.append(dict(world=list(hit),native=[x,py],normal=list(normal),distance_to_root=(hit-center).length))
out=R/'tree21-support-probe-v1.json';assert not out.exists();out.write_text(json.dumps(dict(model_sha256=sha(w/'model.blend'),root_center=list(center),surface_samples=records,scope='Measured nearby archived bank support only, not final terrain approval'),indent=2)+'\n')
print(json.dumps(dict(samples=len(records),root_center=list(center),closest=sorted(records,key=lambda p:p['distance_to_root'])[:12])),flush=True)
