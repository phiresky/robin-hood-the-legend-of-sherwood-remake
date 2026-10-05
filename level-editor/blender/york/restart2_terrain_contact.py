"""Inspect a saved York candidate against its original evaluated terrain."""
import argparse
import hashlib
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'level-editor/work/york-refinement'
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('workspace', type=Path)
parser.add_argument('output', type=Path)
args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
if args.output.exists():
    raise FileExistsError(args.output)
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
from render_slots import acquire
acquire()
sys.path.insert(0, str(ROOT / 'level-editor/blender/nottingham'))
from freeze_tooling import select_tooling
select_tooling(json.loads((OUT/'tooling/current.json').read_text())['directory'])
import bpy
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from render_multiview_asset import render
sys.path.insert(0, str(Path(__file__).resolve().parent))
from restart2_camera_audit import audit_manifest, labeled_copy

model = args.workspace/'model.blend'
bpy.ops.wm.open_mainfile(filepath=str(model))
config = json.loads((args.workspace/'workspace.json').read_text())
scene = bpy.data.scenes[config['scene_name']]
bpy.context.window.scene = scene
bpy.context.view_layer.update()
working = bpy.data.collections[config['collection_name']]
terrain = [o for o in working.all_objects if o.type == 'MESH' and not o.hide_render
           and o.get('source_node') in ('ground','building-086','building-087')]
if len(terrain) != 3:
    raise ValueError(f'Expected background and both raised terrain receivers, found {len(terrain)}')
graph = bpy.context.evaluated_depsgraph_get()
ground_points, ground_faces = [], []
for terrain_obj in terrain:
    ground = terrain_obj.evaluated_get(graph)
    ground_mesh = ground.to_mesh()
    ground_mesh.calc_loop_triangles()
    offset = len(ground_points)
    ground_points.extend(ground.matrix_world @ v.co for v in ground_mesh.vertices)
    ground_faces.extend([offset+i for i in t.vertices] for t in ground_mesh.loop_triangles)
    ground.to_mesh_clear()
tree = BVHTree.FromPolygons(ground_points,ground_faces,all_triangles=True)
candidate_points = []
for obj in working.all_objects:
    if obj.type != 'MESH' or obj.hide_render or obj.get('asset_group') != config['asset_id']:
        continue
    evaluated = obj.evaluated_get(graph)
    candidate_points.extend(evaluated.matrix_world @ v.co for v in evaluated.data.vertices)
minimum_z = min(v.z for v in candidate_points)
contacts = []
for point in candidate_points:
    if point.z > minimum_z + .02:
        continue
    hit = tree.ray_cast(Vector((point.x,point.y,10000)),Vector((0,0,-1)))[0]
    contacts.append({'candidate_world':list(point),
                     'ground_world':list(hit) if hit is not None else None,
                     'vertical_gap_world':point.z-hit.z if hit is not None else None})
for obj in terrain:
    obj['asset_group'] = config['asset_id']
manifest = json.loads((args.workspace/'inspection-v1/actual-views.json').read_text())
manifest['render_object_names'] = None
manifest['object_names'] += [o.name for o in terrain]
args.output.mkdir(parents=True)
manifest_path = args.output/'views.json'
manifest_path.write_text(json.dumps(manifest,indent=2)+'\n')
camera_audit = audit_manifest(manifest_path)
scene.render.engine = 'CYCLES'
scene.cycles.device = 'CPU'
scene.cycles.samples = 16
scene.cycles.use_denoising = False
scene.render.threads_mode = 'FIXED'
scene.render.threads = 2
scene.render.film_transparent = True
scene.view_settings.view_transform = 'Standard'
scene.view_settings.look = 'None'
scene.view_settings.exposure = 0
scene.view_settings.gamma = 1
render(manifest_path,args.output/'actual',width=384)
tiles = [Image.open(args.output/f'actual/view-{i}-textured.png').convert('RGBA') for i in range(8)]
sheet = Image.new('RGBA',(tiles[0].width*4,tiles[0].height*2))
for i,tile in enumerate(tiles):
    sheet.paste(tile,((i%4)*tile.width,(i//4)*tile.height))
sheet.save(args.output/'textured.png')
labeled_copy(args.output/'textured.png',args.output/'textured-native-labeled.png')
(args.output/'evidence.json').write_text(json.dumps({
    'status':'Awaiting visual terrain-contact review; not a geometry approval',
    'model_sha256':hashlib.sha256(model.read_bytes()).hexdigest(),
    'scope':'Eight frozen candidate cameras with the original scene terrain; no copied library transforms, no transformed or substituted ground.',
    'terrain_objects':[o.name for o in terrain],
    'foundation_vertical_samples':contacts,
    'camera_audit':camera_audit,
    'limitations':['Adjacent buildings excluded for these contact views; native joint review remains separate.','Native terrain is a reconstructed ground receiver, not an inferred new support.']},indent=2)+'\n')
