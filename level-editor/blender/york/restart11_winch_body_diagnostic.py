"""Inspect source-space mechanism parts and classify endpoint coverage misses."""
import hashlib
import json
import math
import sys
from pathlib import Path
ROOT = Path(__file__).resolve().parents[3]
WORK = ROOT / 'level-editor/work/york-refinement'
OUT = WORK / 'restart2/winch-body-source-audit-v1'
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image, ImageDraw
s, c = math.sin(math.radians(35)), math.cos(math.radians(35))
back = Vector((0, -c, s))
def project(v):
    return (v.x, -v.y * s - v.z * c)
def tree(objects):
    vs, fs, owners = [], [], []
    for o in objects:
        off = len(vs)
        vs.extend(o.matrix_world @ v.co for v in o.data.vertices)
        for face in o.data.polygons:
            fs.append(tuple(off + i for i in face.vertices)); owners.append(o.name)
    return BVHTree.FromPolygons(vs, fs), owners
source = WORK / 'geometry-pass-01/native-state-source-v1'
record = next(r for r in json.loads((source / 'manifest.json').read_text())['records'] if r['id'] == 'patch-004')
audit = json.loads((WORK / 'restart2/winch-room-physical-v8/native-first-hit-audit.json').read_text())
results=[]
for state, index in [('transition-00', 0), ('transition-44', 44)]:
    path = WORK / 'restart2/winch-room-physical-v8' / state / 'model.blend'
    bpy.ops.wm.open_mainfile(filepath=str(path)); bpy.context.view_layer.update()
    parts = [o for o in bpy.context.scene.objects if o.type == 'MESH' and not o.hide_render and o.get('native_patch') == 'patch-004']
    bvh, owners = tree(parts)
    frame = record['rows'][1]['frames'][index]
    rgba = Image.open(source / frame['image']).convert('RGBA')
    sheet = Image.new('RGB', (780, 930), (45, 45, 45))
    im = rgba.resize((390,930),Image.Resampling.NEAREST)
    sheet.paste(im,(0,0),im); sheet.paste(im,(390,0),im)
    draw = ImageDraw.Draw(sheet)
    def xy(p): return (390+(p[0]-frame['bbox'][0])*10, (p[1]-frame['bbox'][1])*10)
    body = []
    for n, o in enumerate(sorted(parts,key=lambda o:o.name)):
        if not o.name.startswith(('Angled','Frame','Broad','Crank axle')): continue
        zlo = min(v.co.z for v in o.data.vertices); zhi=max(v.co.z for v in o.data.vertices)
        a=o.matrix_world @ Vector((0,0,zlo)); b=o.matrix_world @ Vector((0,0,zhi))
        pa,pb=project(a),project(b)
        color=(80,255,80) if o.name.startswith(('Angled','Frame')) else (80,180,255)
        draw.line([xy(pa),xy(pb)], fill=color, width=2)
        draw.text(xy(pb),str(len(body)), fill=(255,255,255))
        body.append({'id':len(body),'name':o.name,'world_endpoints':[list(a),list(b)],'source_endpoints':[pa,pb]})
    stateaudit=next(r for r in audit['states'] if r['state']==state)
    misses=[]
    for x,y,first in stateaudit['eroded_core_not_winch']:
        hit=bvh.ray_cast(Vector((x+.5,-(y+.5)/s,0))+back*10000,-back)
        misses.append({'pixel':[x,y],'context_first':first,'mechanism_behind':owners[hit[2]] if hit[2] is not None else None})
    sheet.save(OUT/f'{state}-body-axes.png')
    results.append({'state':state,'source_model_sha256':hashlib.sha256(path.read_bytes()).hexdigest(),'body_axes':body,'core_misses':misses})
(OUT/'body-axis-report.json').write_text(json.dumps({'scope':'Read-only source-space diagnostics; no model changes or approvals.','states':results},indent=2)+'\n')
print('WINCH BODY DIAGNOSTIC COMPLETE')
