"""Private revealed-room furniture hypothesis above the hall's upper floor."""
import hashlib
import json
import math
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'level-editor/work/york-refinement'
DEST = OUT / 'restart2/hall-furniture-v1'
if DEST.exists():
    raise FileExistsError(DEST)
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
import bmesh
from mathutils import Matrix

source = OUT / 'geometry-pass-01/assets/york-castle-great-hall/model.blend'
bpy.ops.wm.open_mainfile(filepath=str(source))
DEST.mkdir(parents=True)
native = json.loads((OUT / 'baseline/york.rhp.json').read_text())
sine, cosine = math.sin(math.radians(35)), math.cos(math.radians(35))
nodes = {f'building-{n}' for n in range(824,830)}
objects = [o for o in bpy.data.collections['york Working'].all_objects if o.type=='MESH' and not o.hide_render]

def fingerprint(o):
    data = {'vertices':[list(o.matrix_world@v.co) for v in o.data.vertices],
            'faces':[list(f.vertices) for f in o.data.polygons],
            'uv':[[list(d.uv) for d in layer.data] for layer in o.data.uv_layers]}
    return hashlib.sha256(json.dumps(data,sort_keys=True).encode()).hexdigest()

outside = {o.name:fingerprint(o) for o in objects if o.get('source_node') not in nodes}
rows = []
for n in range(824,830):
    matches = [o for o in objects if o.get('source_node')==f'building-{n}']
    if len(matches)!=1 or matches[0].get('asset_group')!='york-castle-great-hall':
        raise ValueError(f'Ambiguous hall furniture {n}')
    obj = matches[0]
    before = fingerprint(obj)
    ps=native['sight_obstacles'][n]['points']
    # Native top footprints establish placement and height. Their full-height
    # occlusion extrusion is not the visible furniture body.
    origin=((ps[0]['x']+ps[2]['x'])/2,(ps[0]['y']+ps[2]['y'])/2)
    u=((ps[1]['x']-ps[0]['x'])/2,(ps[1]['y']-ps[0]['y'])/2)
    v=((ps[3]['x']-ps[0]['x'])/2,(ps[3]['y']-ps[0]['y'])/2)
    top=sum(p['z_top'] for p in ps)/4
    floor=225.001
    vertices,faces=[],[]
    def volume(outline,lo,hi):
        start=len(vertices);count=len(outline)
        for z in (lo,hi):
            for a,b in outline:
                x=origin[0]+a*u[0]+b*v[0];y=origin[1]+a*u[1]+b*v[1]
                vertices.append((x,-y/sine,z/cosine))
        faces.extend([tuple(start+i for i in reversed(range(count))),tuple(start+count+i for i in range(count))])
        for i in range(count):
            k=(i+1)%count;faces.append((start+i,start+k,start+count+k,start+count+i))
    def box(x0,y0,x1,y1,lo,hi):
        volume([(x0,y0),(x1,y0),(x1,y1),(x0,y1)],lo,hi)
    if n==826:
        seat=floor+18
        box(-1,-1,1,1,seat-3,seat)
        # Tall chair back and arms: rear orientation needs joint state review.
        box(-1,.68,1,1,seat,top)
        box(-1,-.8,-.72,.7,seat,seat+10)
        box(.72,-.8,1,.7,seat,seat+10)
        for x in (-.82,.82):
            for y in (-.82,.82):box(x-.12,y-.12,x+.12,y+.12,floor,seat-3)
    elif n in (825,827):
        volume([(math.cos(i*math.tau/16),math.sin(i*math.tau/16)) for i in range(16)],top-2,top)
        volume([(.23*math.cos(i*math.tau/8),.23*math.sin(i*math.tau/8)) for i in range(8)],floor+1.5,top-2)
        box(-.6,-.6,.6,.6,floor,floor+1.5)
    else:
        box(-1,-1,1,1,top-2,top)
        for x in (-.82,.82):
            for y in (-.82,.82):box(x-.11,y-.11,x+.11,y+.11,floor,top-2)
    mesh=bpy.data.meshes.new(obj.name+' room furniture')
    mesh.from_pydata(vertices,[],faces);mesh.update()
    bm=bmesh.new();bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
    if any(not e.is_manifold for e in bm.edges):raise ValueError(f'Open furniture component {n}')
    bm.to_mesh(mesh);bm.free()
    for material in obj.data.materials:mesh.materials.append(material)
    mesh.uv_layers.new(name='UVMap')
    obj.data=mesh;obj.parent=None;obj.matrix_world=Matrix.Identity(4)
    rows.append({'source_node':obj['source_node'],'before':before,'after':fingerprint(obj),
                 'floor_game_z':floor,'top_game_z':top,'vertices':len(vertices),'faces':len(faces)})
if outside!={o.name:fingerprint(o) for o in objects if o.get('source_node') not in nodes}:
    raise ValueError('Changed a mesh outside the six furniture receivers')
bpy.context.window.scene=bpy.data.scenes['york Refinement']
bpy.context.preferences.filepaths.save_version=0
model=DEST/'model.blend'
bpy.ops.wm.save_as_mainfile(filepath=str(model),compress=True)
(DEST/'geometry.json').write_text(json.dumps({'status':'HOLD: private furniture geometry; revealed projection and actual state review pending',
    'model_sha256':hashlib.sha256(model.read_bytes()).hexdigest(),'changes':rows,'outside_preserved':len(outside),
    'source_evidence':['hall-furniture-source-labels.png','hall-props-828-829-detail.png'],
    'inference':['Hidden legs and pedestal shape.','Upper floor at native volume 769 top, 225.001 game units.',
                 'Throne back orientation requires revealed-state inspection.'],
    'known_limitations':['No UV projection yet.','Adjacent fireplace belongs to the tower and remains unchanged.',
                         'Cover, wall and floor state receivers require independent refinement.']},indent=2)+'\n')
print(model)
