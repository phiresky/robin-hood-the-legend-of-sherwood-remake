"""Balance native opaque coverage against genuine enclosed aperture preservation."""
from collections import deque
import hashlib
import json
import math
from pathlib import Path
import sys

ROOT=Path(__file__).resolve().parents[3]
WORK=ROOT/'level-editor/work/york-refinement'
version='winch-room-physical-v5'
DEST=WORK/'restart2/winch-hole-owners-v1'
if DEST.exists():raise FileExistsError(DEST)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image,ImageFilter

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
    row,index=1,44
    frame=record['rows'][row]['frames'][index];alpha=Image.open(SRC/frame['image']).getchannel('A');core=alpha.filter(ImageFilter.MinFilter(3));parts=holes(alpha)
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

path=WORK/f'restart2/{version}/transition-44/model.blend'
bpy.ops.wm.open_mainfile(filepath=str(path));bpy.context.view_layer.update()
points,apertures=native_points('applied')
objects=[o for o in bpy.context.scene.objects if o.type=='MESH' and not o.hide_render]
trees={o.name:tree([o]) for o in objects};rows=[]
for origin,opaque,core,center,pixel in points:
 if not center:continue
 hits=[]
 for name,t in trees.items():
  hit=t.ray_cast(origin,-back)
  if hit[0] is not None:hits.append({'object':name,'distance':hit[3],'location':list(hit[0]),'winch':bpy.context.scene.objects[name].get('native_patch')=='patch-004'})
 rows.append({'source_pixel':pixel,'global_pixel':[2390+pixel[0],882+pixel[1]],'hits':sorted(hits,key=lambda h:h['distance'])})
DEST.mkdir(parents=True);(DEST/'report.json').write_text(json.dumps({'model_sha256':hashlib.sha256(path.read_bytes()).hexdigest(),'holes':rows},indent=2)+'\n');print(json.dumps(rows,indent=2))
