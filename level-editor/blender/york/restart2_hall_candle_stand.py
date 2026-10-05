"""Private authored candle-stand receiver from native mask 635 and animation 000."""
import hashlib
import json
import math
from pathlib import Path
import sys

ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/york-refinement'
DEST=OUT/'restart2/hall-furniture-v6'
if DEST.exists():raise FileExistsError(DEST)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
import bmesh
from mathutils import Vector

source=OUT/'restart2/hall-furniture-v5/model.blend'
bpy.ops.wm.open_mainfile(filepath=str(source))
DEST.mkdir(parents=True)
collection=bpy.data.collections['york Working']
node='scenery-york-great-hall-candle-stand'
if any(o.get('source_node')==node for o in collection.all_objects):raise ValueError('Candle receiver already exists')
sine,cosine=math.sin(math.radians(35)),math.cos(math.radians(35))
vertices,faces=[],[]
def point(p):return Vector((p[0],-p[1]/sine,p[2]/cosine))
def rod(start,end,radius,sides=10):
    a,b=point(start),point(end);axis=(b-a).normalized()
    auxiliary=Vector((1,0,0)) if abs(axis.x)<.9 else Vector((0,1,0))
    u=axis.cross(auxiliary).normalized()*radius;v=axis.cross(u).normalized()*radius
    index=len(vertices)
    for center in (a,b):
        vertices.extend(tuple(center+u*math.cos(i*math.tau/sides)+v*math.sin(i*math.tau/sides)) for i in range(sides))
    faces.extend([tuple(index+i for i in reversed(range(sides))),tuple(index+sides+i for i in range(sides))])
    faces.extend((index+i,index+(i+1)%sides,index+sides+(i+1)%sides,index+sides+i) for i in range(sides))
rod((2880,851,231),(2880,851,274),.65)
for foot in ((2873,849,225.001),(2887,853,225.001),(2879,854,225.001)):
    rod(foot,(2880,851,234),.8)
for lo,hi,radius in ((232,235,1.7),(244,247,2.0),(246,249,1.2)):
    rod((2880,851,lo),(2880,851,hi),radius)
candles=[(2871,849,270,279),(2879,848,274,283),(2889,853,272,281)]
for x,y,base,top in candles:
    rod((2880,851,266),(x,y,base-1),.65)
    rod((x,y,base-1),(x,y,base),1.55)
    rod((x,y,base),(x,y,top),1.05)
mesh=bpy.data.meshes.new('York great hall candle stand')
mesh.from_pydata(vertices,[],faces);mesh.update()
bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
if any(not e.is_manifold for e in bm.edges):raise ValueError('Open candle stand component')
bm.to_mesh(mesh);bm.free();mesh.uv_layers.new(name='UVMap')
obj=bpy.data.objects.new('Castle great hall / Three-candle stand',mesh);collection.objects.link(obj)
obj['source_node']=node;obj['asset_group']='york-castle-great-hall';obj['asset_name']='Castle great hall';obj['part_name']='Three-candle stand'
obj['authored_scenery']=True
bpy.context.window.scene=bpy.data.scenes['york Refinement']
bpy.context.preferences.filepaths.save_version=0
model=DEST/'model.blend';bpy.ops.wm.save_as_mainfile(filepath=str(model),compress=True)
(DEST/'geometry.json').write_text(json.dumps({'status':'HOLD: private authored receiver; not a catalog change or animation integration',
    'source_model_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'model_sha256':hashlib.sha256(model.read_bytes()).hexdigest(),
    'node':node,'mask_index':635,'native_animation':'animation-000','floor_game_z':225.001,
    'inferred':['Depth, rod sections, branch bends and tripod spacing from the single native view.'],
    'observed':['Three candle stems, central pole, two decorative collars, spreading base.'],
    'remaining':['Masked native and actual multiview source-fit review.','Preserve native flame-frame RGBA and timing on a separate receiver.','Full room state and canonical scenery ownership proposal.'],
    'catalog_part_proposal':{'node':node,'name':'Three-candle stand'},'obstacle_created':False},indent=2)+'\n')
