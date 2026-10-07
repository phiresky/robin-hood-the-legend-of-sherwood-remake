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
DEST=WORK/'restart2/winch-chain-fit-v1'
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
parts=[o for o in bpy.context.scene.objects if o.type=='MESH' and not o.hide_render and o.get('native_patch')=='patch-004']
context=tree([o for o in bpy.context.scene.objects if o.type=='MESH' and not o.hide_render and o not in parts])
fixed=[o for o in parts if not o.name.startswith('Suspended chain')]
points,apertures=native_points('applied')
# Front-facing links repeat every seven native vertical pixels; interlocked
# edge-facing links lie halfway between. Hidden chain depth is an inference.
def chain_tree(radius,tube,vertical_scale,phase):
    verts=[];faces=[]
    for o in fixed:
        start=len(verts);verts.extend(o.matrix_world@v.co for v in o.data.vertices)
        faces.extend(tuple(start+i for i in p.vertices) for p in o.data.polygons)
    for x,y,lo,hi,front_z in ((2400.5,1059,113,177,171.5),(2410.5,1064,111,182,174.5)):
        for n in range(-2,24):
            z=front_z-n*3.5+phase
            if not lo<=z<=hi:continue
            center=Vector((x,-y/s,z/c));start=len(verts)
            for a in range(16):
                theta=a*math.tau/16
                for b in range(6):
                    phi=b*math.tau/6
                    radial=radius+tube*math.cos(phi)
                    u=radial*math.cos(theta);v=radial*math.sin(theta)*vertical_scale;depth=tube*math.sin(phi)
                    verts.append(center+Vector((u if n%2==0 else depth,depth if n%2==0 else u,v)))
            for a in range(16):
                for b in range(6):faces.append(tuple(start+aa*6+bb for aa,bb in ((a,b),((a+1)%16,b),((a+1)%16,(b+1)%6),(a,(b+1)%6))))
    return BVHTree.FromPolygons(verts,faces)
results=[]
for radius in (1.2,1.4,1.6):
 for tube in (.45,.6,.75):
  for stretch in (1.4,1.7):
   for phase in (-.5,0,.5):
    counts=evaluate(chain_tree(radius,tube,stretch,phase),context,points)
    score=counts['opaque_core_missed']*5+counts['hole_centers_filled']*60+counts['opaque_missed']*.2+counts['hole_pixels_filled']*.5
    results.append({'radius':radius,'tube':tube,'vertical_scale':stretch,'phase':phase,'counts':counts,'score':score})
results.sort(key=lambda r:r['score']);DEST.mkdir(parents=True)
(DEST/'sweep.json').write_text(json.dumps({'status':'Private chain spacing diagnostic; no geometry changes saved','source_model_sha256':hashlib.sha256(path.read_bytes()).hexdigest(),'source_holes':apertures,'candidates':results},indent=2)+'\n')
print(json.dumps({'top':results[:8]},indent=2))
