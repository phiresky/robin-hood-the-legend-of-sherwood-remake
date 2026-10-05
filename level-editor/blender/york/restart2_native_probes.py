"""Report evaluated geometry along selected native source-camera rays."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('model', type=Path)
parser.add_argument('output', type=Path)
parser.add_argument('--point', action='append', nargs=2, type=float, required=True)
args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
if args.output.exists():
    raise FileExistsError(args.output)
sys.path.insert(0, str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree

bpy.ops.wm.open_mainfile(filepath=str(args.model.resolve()))
scene = bpy.data.scenes['york Refinement']
bpy.context.window.scene = scene
bpy.context.view_layer.update()
graph = bpy.context.evaluated_depsgraph_get()
trees = []
for obj in bpy.data.collections['york Working'].all_objects:
    if obj.type != 'MESH' or obj.hide_render:
        continue
    evaluated = obj.evaluated_get(graph)
    mesh = evaluated.to_mesh()
    mesh.calc_loop_triangles()
    points = [evaluated.matrix_world @ v.co for v in mesh.vertices]
    faces = [tuple(t.vertices) for t in mesh.loop_triangles]
    if faces:
        trees.append((obj, BVHTree.FromPolygons(points, faces, all_triangles=True)))
    evaluated.to_mesh_clear()
sine, cosine = math.sin(math.radians(35)), math.cos(math.radians(35))
rows = []
for x, y in args.point:
    origin = Vector((x, -y*sine-10000*cosine, -y*cosine+10000*sine))
    direction = Vector((0, cosine, -sine))
    hits = []
    for obj, tree in trees:
        point, normal, face, distance = tree.ray_cast(origin, direction)
        if point is None:
            continue
        hits.append({'object': obj.name, 'source_node': obj.get('source_node'),
                     'asset_group': obj.get('asset_group'), 'distance': distance,
                     'world': list(point), 'game': [point.x, -point.y*sine, point.z*cosine]})
    rows.append({'source_pixel': [x, y], 'hits_front_to_back': sorted(hits, key=lambda h:h['distance'])})
args.output.parent.mkdir(parents=True, exist_ok=True)
args.output.write_text(json.dumps({
    'scope': 'Original native orthographic camera rays through evaluated working meshes; opaque geometry only, not alpha or semantic ownership.',
    'model_sha256': hashlib.sha256(args.model.read_bytes()).hexdigest(),
    'probes': rows}, indent=2)+'\n')
print(json.dumps(rows))
