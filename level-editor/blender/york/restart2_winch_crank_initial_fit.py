"""Balance native opaque coverage against genuine enclosed aperture preservation."""
from collections import deque
import hashlib
import json
import math
from pathlib import Path
import sys

ROOT=Path(__file__).resolve().parents[3]
WORK=ROOT/'level-editor/work/york-refinement'
version='winch-room-physical-v7'
DEST=WORK/'restart2/winch-crank-balanced-initial-fit-v1'
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
    row,index=1,0
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

path=WORK/f'restart2/{version}/transition-00/model.blend'
bpy.ops.wm.open_mainfile(filepath=str(path));bpy.context.view_layer.update()
parts=[o for o in bpy.context.scene.objects if o.type=='MESH' and not o.hide_render and o.get('native_patch')=='patch-004']
context=tree([o for o in bpy.context.scene.objects if o.type=='MESH' and not o.hide_render and o not in parts]);points,apertures=native_points('applied')
def world(x,y,z):return Vector((x,-y/s,z/c))
center=world(2410,1064,104);rear=world(2402,1050,104);axis=(center-rear).normalized();lateral=axis.cross(Vector((0,0,1))).normalized()
spokes=sorted((o for o in parts if o.name.startswith('Crank spoke')),key=lambda o:o.name)
assert len(spokes)==8
# Only rigid spoke phase, length and physical thickness vary. Axis, frame,
# drum, chain openings and all context stay fixed; no model is saved.
frame=record['rows'][1]['frames'][0];alpha=Image.open(SRC/frame['image']).getchannel('A');padded=ImageOps.expand(alpha,border=5,fill=0);dilated=padded.filter(ImageFilter.MaxFilter(3));exterior=[]
for y in range(padded.height):
 for x in range(padded.width):
  sx,sy=frame['bbox'][0]+x-5+.5,frame['bbox'][1]+y-5+.5
  if sy>=944 and dilated.getpixel((x,y))<128:exterior.append(Vector((sx,-sy/s,0))+back*10000)
results=[]
for phase in (-15,-10,-5,0,5,10,15):
 for radius in (17,18.5,20):
  for thickness in (.9,1.2,1.5):
   for i,o in enumerate(spokes):
    angle=i*math.tau/8+math.radians(phase);end=center+(lateral*math.cos(angle)+Vector((0,0,1))*math.sin(angle))*radius
    o.location=(center+end)/2;o.rotation_euler=(end-center).to_track_quat('Z','Y').to_euler();o.scale=Vector((thickness/.9,thickness/.9,radius/17))
   bpy.context.view_layer.update();part_tree=tree(parts);counts=evaluate(part_tree,context,points)
   excess=0
   for origin in exterior:
    hit=part_tree.ray_cast(origin,-back);other=context.ray_cast(origin,-back)
    excess+=int(hit[0] is not None and (other[0] is None or hit[3]<other[3]-1e-5))
   counts['body_exterior_beyond_one_pixel']=excess
   score=counts['opaque_core_missed']*5+counts['hole_centers_filled']*500+counts['opaque_missed']*.2+counts['hole_pixels_filled']*.5+counts['body_exterior_beyond_one_pixel']*5
   results.append({'phase_degrees':phase,'radius':radius,'thickness':thickness,'counts':counts,'score':score})
results.sort(key=lambda r:r['score']);DEST.mkdir(parents=True);(DEST/'sweep.json').write_text(json.dumps({'status':'Diagnostic only; source silhouette excess requires separate check before acceptance','source_model_sha256':hashlib.sha256(path.read_bytes()).hexdigest(),'candidates':results},indent=2)+'\n');print(json.dumps(results[:8],indent=2))
