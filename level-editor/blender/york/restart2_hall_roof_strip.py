"""Prototype the source-visible retained roof strip, independently of room states."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'level-editor/work/york-refinement'
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--fraction', type=float, default=.22)
parser.add_argument('--version', default='hall-roof-strip-v1')
args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:] if '--' in sys.argv else [])
if not 0 < args.fraction < 1:
    raise ValueError('Roof strip must retain a strict subset of the roof')
destination = OUT / 'restart2' / args.version
if destination.exists():
    raise FileExistsError(destination)
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
from render_slots import acquire
acquire()
sys.path.insert(0, str(ROOT / 'level-editor/blender/nottingham'))
from freeze_tooling import select_tooling
select_tooling(json.loads((OUT / 'tooling/current.json').read_text())['directory'])
import bpy
import bmesh
from mathutils import Matrix
from source_projection_bake import bake

source_model = OUT / 'restart2/hall-shell-v2/model.blend'
bpy.ops.wm.open_mainfile(filepath=str(source_model))
bpy.context.view_layer.update()
targets = [o for o in bpy.data.collections['york Working'].all_objects
           if o.type == 'MESH' and not o.hide_render and o.get('source_node') == 'building-799']
if len(targets) != 1:
    raise ValueError('Expected exactly one retained hall roof')
obj = targets[0]
native = json.loads((OUT / 'baseline/york.rhp.json').read_text())
points = [(p['x'],p['y'],p['z_top']) for p in native['sight_obstacles'][799]['points']]
lerp = lambda a,b: tuple(x + args.fraction * (y-x) for x,y in zip(a,b))
profile = [points[0], lerp(points[0],points[1]), lerp(points[3],points[2]), points[3]]
sine, cosine = math.sin(math.radians(35)), math.cos(math.radians(35))
vertices = [(x,-y/sine,(z-dz)/cosine) for dz in (2.5,0) for x,y,z in profile]
faces = [(3,2,1,0),(4,5,6,7)] + [(i,(i+1)%4,(i+1)%4+4,i+4) for i in range(4)]
mesh = bpy.data.meshes.new('Hall retained roof strip')
mesh.from_pydata(vertices, [], faces)
bm = bmesh.new(); bm.from_mesh(mesh)
bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
if any(not edge.is_manifold for edge in bm.edges):
    raise ValueError('Retained roof strip is open')
bm.to_mesh(mesh); bm.free()
for material in obj.data.materials:
    mesh.materials.append(material)
mesh.uv_layers.new(name='UVMap')
obj.data = mesh; obj.parent = None; obj.matrix_world = Matrix.Identity(4)
destination.mkdir(parents=True)
bake('york', OUT/'baseline/revealed.png', destination/'projection.json',
     receiver_nodes=['building-799'], projection_label='retained-roof-strip-diagnostic',
     texels_per_unit=2, preserve_authored=False)
obj['asset_group'] = 'private-york-retained-hall-roof'
bpy.context.preferences.filepaths.save_version = 0
bpy.ops.wm.save_as_mainfile(filepath=str(destination/'model.blend'), compress=True)
config = json.loads((OUT/'geometry-pass-01/assets/york-castle-great-hall/workspace.json').read_text())
config.update(asset_id='private-york-retained-hall-roof', source_path=str(OUT/'baseline/revealed.png'))
(destination/'workspace.json').write_text(json.dumps(config,indent=2)+'\n')
(destination/'geometry.json').write_text(json.dumps({
    'status':'HOLD: retained roof strip hypothesis only; not a complete revealed room or native state implementation',
    'source_model_sha256':hashlib.sha256(source_model.read_bytes()).hexdigest(),
    'retained_cross_roof_fraction':args.fraction,
    'roof_profile_game':profile,
    'roof_profile_source':[[x,y-z] for x,y,z in profile],
    'observed':'Revealed artwork retains a narrow tiled strip beside the rear timber wall; the tower hides its ends.',
    'inferred':'Constant strip width and 2.5 game-unit thickness; fraction must pass native source inspection.',
    'remaining':['Patch001/002 independent state geometry','Front cut wall and complete room','Precise revealed roof ownership mask']},indent=2)+'\n')
