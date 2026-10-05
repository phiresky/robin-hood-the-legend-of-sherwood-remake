"""Private revealed-room shell hypothesis with a low front wall and furniture."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'level-editor/work/york-refinement'
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--version', default='hall-room-v1')
args = parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else [])
destination = OUT/'restart2'/args.version
if destination.exists():
    raise FileExistsError(destination)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
sys.path.insert(0,str(ROOT/'level-editor/blender/nottingham'))
from freeze_tooling import select_tooling
select_tooling(json.loads((OUT/'tooling/current.json').read_text())['directory'])
import bpy
import bmesh
from mathutils import Matrix
from source_projection_bake import bake

source = OUT/'restart2/hall-shell-v2/covered-workspace/model.blend'
bpy.ops.wm.open_mainfile(filepath=str(source))
scene = bpy.data.scenes['york Refinement']
bpy.context.window.scene = scene
bpy.context.view_layer.update()
collection = bpy.data.collections['york Working']
asset = 'york-castle-great-hall'
sine, cosine = math.sin(math.radians(35)), math.cos(math.radians(35))


def fingerprint(obj):
    return hashlib.sha256(json.dumps({
        'vertices':[list(obj.matrix_world @ v.co) for v in obj.data.vertices],
        'faces':[list(f.vertices) for f in obj.data.polygons],
        'uv':[[list(d.uv) for d in layer.data] for layer in obj.data.uv_layers],
        'materials':[m.name if m else None for m in obj.data.materials]},sort_keys=True).encode()).hexdigest()


outside = {o.name:fingerprint(o) for o in collection.all_objects if o.type=='MESH' and o.get('asset_group')!=asset}
targets = {o['source_node']:o for o in collection.all_objects
           if o.type=='MESH' and not o.hide_render and o.get('asset_group')==asset}
shell = json.loads((OUT/'restart2/hall-shell-v2/geometry.json').read_text())
profiles = {r['source_node']:r['profile_game'] for r in shell['changes']}
changes = []


def volume(node, profile, bottom):
    obj = targets[node]
    count = len(profile)
    low = bottom if isinstance(bottom,list) else [bottom]*count
    vertices = [(x,-y/sine,z/cosine) for (x,y,_),z in zip(profile,low)]
    vertices += [(x,-y/sine,z/cosine) for x,y,z in profile]
    faces = [tuple(reversed(range(count))),tuple(count+i for i in range(count))]
    faces += [(i,(i+1)%count,(i+1)%count+count,i+count) for i in range(count)]
    mesh = bpy.data.meshes.new(node+' revealed room control')
    mesh.from_pydata(vertices,[],faces)
    bm=bmesh.new();bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
    if any(not e.is_manifold for e in bm.edges):
        raise ValueError(f'Open revealed room component {node}')
    bm.to_mesh(mesh);bm.free()
    for material in obj.data.materials:
        mesh.materials.append(material)
    mesh.uv_layers.new(name='UVMap')
    obj.data=mesh;obj.parent=None;obj.matrix_world=Matrix.Identity(4)
    changes.append({'node':node,'profile_game':profile,'bottom_game_z':low})


# The front occlusion proxy is much thicker than the visible cut wall.
# These source-led plan coordinates are a hypothesis, with 1–3 pixel edge
# uncertainty. Preserve both exact shared hall/tower partition endpoints.
base = profiles['building-769']
floor = [base[0],(2915,933,225.001),(2859,950,225.001),(2747,833,225.001),
         (2760.2766,823.65656,225.001),*base[4:]]
volume('building-769',floor,90.00101)
wall = [(2747,833,237.001),(2859,950,237.001),(2915,933,237.001),
        (2923,919,237.001),(2864.5054,936.39105,237.001),(2760.2766,823.65656,237.001)]
volume('building-793',wall,225.001)
roof = json.loads((OUT/'restart2/hall-roof-strip-v2/geometry.json').read_text())['roof_profile_game']
volume('building-799',roof,[z-2.5 for x,y,z in roof])

# Link and evaluate donors before examining transforms. Refined furniture
# already uses world-space vertices and has no parent; reject a changed basis.
donor_path = OUT/'restart2/hall-furniture-v6/source-fit/native-identities.blend'
names = [targets[f'building-{n}'].name for n in range(824,830)]
names.append('Castle great hall / Three-candle stand')
with bpy.data.libraries.load(str(donor_path),link=False) as (available,imported):
    if not set(names)<=set(available.objects):
        raise ValueError('Refined furniture donor incomplete')
    imported.objects = names
for donor in imported.objects:
    collection.objects.link(donor)
bpy.context.view_layer.update()
for donor in imported.objects:
    error=max(abs(donor.matrix_world[r][c]-(1 if r==c else 0)) for r in range(4) for c in range(4))
    if donor.parent or error>1e-6:
        raise ValueError('Furniture donor has an unexpected evaluated transform')
    node=donor['source_node']
    if node in targets:
        target=targets[node]
        target.data=donor.data.copy()
        target.parent=None;target.matrix_world=Matrix.Identity(4)
        bpy.data.objects.remove(donor,do_unlink=True)
    else:
        if node!='scenery-york-great-hall-candle-stand':
            raise ValueError('Unexpected scenery donor')
        donor['asset_group']=asset
        targets[node]=donor
destination.mkdir(parents=True)
bake('york',OUT/'baseline/revealed.png',destination/'projection.json',
     receiver_nodes=list(targets),receiver_asset_id=asset,
     projection_label='private-hall-room-hypothesis',texels_per_unit=2,preserve_authored=False)
if outside!={o.name:fingerprint(o) for o in collection.all_objects if o.type=='MESH' and o.get('asset_group')!=asset}:
    raise ValueError('Changed outside hall geometry, UV or material assignment')
bpy.context.preferences.filepaths.save_version=0
bpy.ops.wm.save_as_mainfile(filepath=str(destination/'model.blend'),compress=True)
config=json.loads((OUT/'geometry-pass-01/assets/york-castle-great-hall/workspace.json').read_text())
config.update(source_path=str(OUT/'baseline/revealed.png'),part_ids=list(targets))
(destination/'workspace.json').write_text(json.dumps(config,indent=2)+'\n')
(destination/'geometry.json').write_text(json.dumps({
    'status':'HOLD: room-shell hypothesis, not complete patch 001/002 state integration or approved source ownership',
    'source_model_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),
    'furniture_source_sha256':hashlib.sha256(donor_path.read_bytes()).hexdigest(),
    'changes':changes,'outside_preserved':len(outside),
    'observations':['Revealed source retains a narrow roof strip and low front cut wall.','Foreground wall 647 occludes lower chess-table legs.'],
    'inferred':['Wall top 237.001 versus floor 225.001.','Revised front footprint from observed cut-wall edge; hidden lower continuation remains a hypothesis.'],
    'remaining':['Inspect native/actual source fit before selecting this geometry.','Northwest arch/doorway and independent patch 001/002 conditions.','Adjacent tower fireplace reveal and exact candle flame states.','Precise masked source ownership; covered shell must share any accepted lower footprint change.']},indent=2)+'\n')
