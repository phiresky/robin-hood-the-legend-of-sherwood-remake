"""Balance native opaque coverage against genuine enclosed aperture preservation."""
from collections import deque
import hashlib
import json
import math
from pathlib import Path
import sys

ROOT=Path(__file__).resolve().parents[3]
WORK=ROOT/'level-editor/work/york-refinement'
version='winch-room-physical-v6'
DEST=WORK/'restart2/winch-chain-thickness-v1'
if DEST.exists():raise FileExistsError(DEST)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image,ImageFilter,ImageOps

s,c=math.sin(math.radians(35)),math.cos(math.radians(35));back=Vector((0,-c,s))
SRC=WORK/'geometry-pass-01/native-state-source-v1'
record=next(r for r in json.loads((SRC/'manifest.json').read_text())['records'] if r['id']=='patch-004')
def holes(alpha):
    w,h=alpha.size;remaining={(x,y) for y in range(h) for x in range(w) if alpha.getpixel((x,y))<128};result=[]
    while remaining:
        seed=remaining.pop();q=deque([seed]);part=[seed]
        while q:
            x,y=q.popleft()
            for p in ((x-1,y),(x+1,y),(x,y-1),(x,y+1)):
                if p in remaining:remaining.remove(p);part.append(p);q.append(p)
        if any(x in (0,w-1) or y in (0,h-1) for x,y in part):continue
        cx=sum(p[0] for p in part)/len(part);cy=sum(p[1] for p in part)/len(part)
        center=min(part,key=lambda p:((p[0]-cx)**2+(p[1]-cy)**2,p))
        result.append({'pixels':sorted(part),'area':len(part),'center':center})
    return result
def tree(objects):
    vertices=[];faces=[]
    for o in objects:
        start=len(vertices);vertices.extend(o.matrix_world@v.co for v in o.data.vertices)
        faces.extend(tuple(start+i for i in p.vertices) for p in o.data.polygons)
    return BVHTree.FromPolygons(vertices,faces)
def native_points(state):
    row,index=1,(0 if state=='initial' else 44)
    frame=record['rows'][row]['frames'][index];alpha=Image.open(SRC/frame['image']).getchannel('A');core=ImageOps.expand(alpha,border=1,fill=0).filter(ImageFilter.MinFilter(3)).crop((1,1,alpha.width+1,alpha.height+1));parts=holes(alpha)
    hole_pixels={tuple(p) for h in parts for p in h['pixels']}
    # Every enclosed hole of >=3 source pixels contributes one center even if
    # its width is only one pixel; morphological erosion cannot erase this test.
    centers={tuple(h['center']) for h in parts if h['area']>=3}
    points=[]
    for y in range(alpha.height):
        for x in range(alpha.width):
            if alpha.getpixel((x,y))>=128 or (x,y) in hole_pixels:
                sx,sy=frame['bbox'][0]+x+.5,frame['bbox'][1]+y+.5
                points.append((Vector((sx,-sy/s,0))+back*10000,alpha.getpixel((x,y))>=128,core.getpixel((x,y))>=128,(x,y) in centers,(x,y)))
    return points,parts
def evaluate(gate_tree,context_tree,points):
    counts={'opaque_missed':0,'opaque_core_missed':0,'hole_pixels_filled':0,'hole_centers_filled':0};filled=[]
    for origin,opaque,core,center,pixel in points:
        gh=gate_tree.ray_cast(origin,-back);ch=context_tree.ray_cast(origin,-back)
        hits=gh[0] is not None and (ch[0] is None or gh[3]<ch[3]-1e-5)
        if opaque and not hits:counts['opaque_missed']+=1;counts['opaque_core_missed']+=int(core)
        if not opaque and hits:
            counts['hole_pixels_filled']+=1;counts['hole_centers_filled']+=int(center);filled.append(pixel)
    return counts

path=WORK/f'restart2/{version}/transition-44/model.blend';bpy.ops.wm.open_mainfile(filepath=str(path));bpy.context.view_layer.update()
parts=[o for o in bpy.context.scene.objects if o.type=='MESH' and not o.hide_render and o.get('native_patch')=='patch-004'];initial=[o for o in parts if not o.name.startswith('Travelling round part')]
context=tree([o for o in bpy.context.scene.objects if o.type=='MESH' and not o.hide_render and o not in parts]);final_points,_=native_points('applied');initial_points,_=native_points('initial')
chains=[o for o in parts if o.name.startswith('Suspended chain')];original={o.name:[v.co.copy() for v in o.data.vertices] for o in chains};results=[]
for major in (1.2,1.3,1.4,1.5):
 for tube in (.55,.65,.75):
  for o in chains:
   for vertex,old in zip(o.data.vertices,original[o.name]):
    radial=math.hypot(old.x,old.y);new_radius=major+(radial-1.45)*tube/.55;vertex.co=Vector((old.x*new_radius/radial,old.y*new_radius/radial,old.z*tube/.55))
   o.data.update()
  bpy.context.view_layer.update();counts=evaluate(tree(parts),context,final_points);early=evaluate(tree(initial),context,initial_points)
  score=sum(r['opaque_core_missed']*5+r['hole_centers_filled']*500+r['opaque_missed']*.2+r['hole_pixels_filled']*.5 for r in (counts,early))
  results.append({'major_radius':major,'tube_radius':tube,'final':counts,'initial':early,'score':score})
results.sort(key=lambda r:r['score']);DEST.mkdir(parents=True);(DEST/'sweep.json').write_text(json.dumps({'status':'Private source link thickness diagnostic; no models saved','source_model_sha256':hashlib.sha256(path.read_bytes()).hexdigest(),'initial_scope':'Moving part excluded from ray set because complete early pose is above this source crop; actual room endpoints separately rendered','candidates':results},indent=2)+'\n');print(json.dumps(results[:6],indent=2))
