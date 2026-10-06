"""Balance native opaque coverage against genuine enclosed aperture preservation."""
from collections import deque
import hashlib
import json
import math
from pathlib import Path
import sys

ROOT=Path(__file__).resolve().parents[3]
WORK=ROOT/'level-editor/work/york-refinement'
verify='--verify' in sys.argv
version=sys.argv[sys.argv.index('--verify')+1] if verify else 'gate-geometry-v9'
DEST=WORK/'restart2'/version/'balanced-first-hits.json' if verify else WORK/'restart2/gate-balanced-fit-v1'
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
record=next(r for r in json.loads((SRC/'manifest.json').read_text())['records'] if r['id']=='patch-000')
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
    row,index=(0,0) if state=='covered' else (1,44)
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

contexts={};points={};hole_records={};original=[]
for state in ('covered','raised'):
    points[state],hole_records[state]=native_points(state)
    path=WORK/f'restart2/{version}/{state}/model.blend';bpy.ops.wm.open_mainfile(filepath=str(path));bpy.context.view_layer.update()
    gate=bpy.data.objects['scenery-york-castle-portcullis']
    contexts[state]=tree([o for o in bpy.context.scene.objects if o.type=='MESH' and not o.hide_render and o!=gate])
    if state=='covered':original=[gate.matrix_world@v.co for v in gate.data.vertices];faces=[tuple(p.vertices) for p in gate.data.polygons]
    else:
        assert faces==[tuple(p.vertices) for p in gate.data.polygons]
        assert max(((gate.matrix_world@v.co)-(p+Vector((0,0,57/c)))).length for v,p in zip(gate.data.vertices,original))<1e-4
assert len(original)==17*8
reference_width=max(p.z for p in original[88:96])-min(p.z for p in original[88:96])
results=[]
# Upright section remains the source-aligned v8 adjustment. Only six crossbar
# thicknesses and their shared vertical phase vary; no pixels are reassigned.
for width in ((reference_width,) if verify else (2.0,2.5,3.0,3.5,4.0,4.5,5.0)):
    for phase in ((0,) if verify else (-1.5,-1.0,-.5,0,.5,1.0,1.5)):
        vertices=[p.copy() for p in original]
        for beam in range(11,17):
            center=sum(p.z for p in original[beam*8:beam*8+8])/8
            for i in range(beam*8,beam*8+8):vertices[i].z=center+(original[i].z-center)*width/reference_width+phase/c
        row={'crossbar_width_world':width,'phase_game_z':phase,'states':{}}
        for state,lift in [('covered',0),('raised',57)]:
            gt=BVHTree.FromPolygons([p+Vector((0,0,lift/c)) for p in vertices],faces)
            row['states'][state]=evaluate(gt,contexts[state],points[state])
        row['score']=sum(r['opaque_core_missed']*5+r['hole_centers_filled']*20+r['opaque_missed']*.2+r['hole_pixels_filled']*.5 for r in row['states'].values())
        results.append(row)
results.sort(key=lambda r:r['score'])
if not verify:DEST.mkdir(parents=True)
report={'status':'Exact unchanged saved candidate audit' if verify else 'Bounded diagnostic sweep, not automatic geometry approval','basis_models':version,'model_sha256':{state:hashlib.sha256((WORK/f'restart2/{version}/{state}/model.blend').read_bytes()).hexdigest() for state in ('covered','raised')},'hole_definition':'4-connected enclosed transparent components; area>=3 center pixel tested separately, including1pixel-wide apertures','holes':hole_records,'candidates':results,'scoring':'Ordering aid only: opaque core misses5, enclosed-hole center loss20, all opaque misses0.2, transparent hole fills0.5. Both states included.'}
(DEST if verify else DEST/'sweep.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({'hole_components':{k:len(v) for k,v in hole_records.items()},'top':results[:8]},indent=2))
