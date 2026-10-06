"""Audit native sprite first hits on the saved endpoint geometry without changing it."""
import hashlib
import json
import math
from pathlib import Path
import sys

ROOT=Path(__file__).resolve().parents[3]
WORK=ROOT/'level-editor/work/york-refinement'
BASE=WORK/'restart2'/(sys.argv[sys.argv.index('--')+1] if '--' in sys.argv else 'winch-room-probe-v1')
DEST=BASE/'native-first-hit-audit.json'
if DEST.exists():raise FileExistsError(DEST)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image,ImageFilter

sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
s,c=math.sin(math.radians(35)),math.cos(math.radians(35))
back=Vector((0,-c,s))
source=WORK/'geometry-pass-01/native-state-source-v1'
record=next(r for r in json.loads((source/'manifest.json').read_text())['records'] if r['id']=='patch-004')
states=[]
for state,row,index in [('transition-44',1,44)]:
    path=BASE/state/'model.blend';digest=sha(path);bpy.ops.wm.open_mainfile(filepath=str(path));bpy.context.view_layer.update()
    scene=bpy.context.scene;verts=[];polys=[];owners=[]
    for o in scene.objects:
        if o.type!='MESH' or o.hide_render:continue
        start=len(verts);verts.extend(o.matrix_world@v.co for v in o.data.vertices)
        for p in o.data.polygons:polys.append(tuple(start+i for i in p.vertices));owners.append(o.get('source_node',o.name) if o.get('native_patch')=='patch-004' else o.name)
    tree=BVHTree.FromPolygons(verts,polys)
    frame=record['rows'][row]['frames'][index];alpha=Image.open(source/frame['image']).getchannel('A');core=alpha.filter(ImageFilter.MinFilter(3))
    counts={};misses=[];core_misses=[];rows=[]
    for y in range(alpha.height):
        for x in range(alpha.width):
            if alpha.getpixel((x,y))<128:continue
            sx,sy=frame['bbox'][0]+x+.5,frame['bbox'][1]+y+.5
            start=Vector((sx,-sy/s,0))+back*10000
            point,normal,face,distance=tree.ray_cast(start,-back)
            owner=owners[face] if face is not None else 'NO_HIT'
            counts[owner]=counts.get(owner,0)+1
            rows.append({'pixel':[int(sx),int(sy)],'owner':owner})
            if owner!='scenery-york-castle-winch':
                misses.append([int(sx),int(sy),owner])
                if core.getpixel((x,y))>=128:core_misses.append([int(sx),int(sy),owner])
    assert sha(path)==digest
    states.append({'state':state,'model_sha256':digest,'source_frame_sha256':sha(source/frame['image']),
                   'pixel_center_first_hits':counts,'not_winch':misses,'eroded_core_not_winch':core_misses})
DEST.write_text(json.dumps({'scope':'Opaque saved triangle first hits at exact native source pixel centers; no material alpha, no coverage waiver, no mutations.',
                           'states':states},indent=2)+'\n')
print(json.dumps([{'state':r['state'],'first_hits':r['pixel_center_first_hits'],'not_winch':len(r['not_winch']),'core_not_winch':len(r['eroded_core_not_winch'])} for r in states]))
