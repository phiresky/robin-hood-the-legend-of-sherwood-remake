"""Compare reopened and evaluated York context against its frozen baseline."""
import argparse
import hashlib
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'level-editor/work/york-refinement'
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('candidate', type=Path)
parser.add_argument('output', type=Path)
parser.add_argument('--exclude-group', action='append', required=True)
args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
if args.output.exists():
    raise FileExistsError(args.output)
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy


def snapshot(path):
    bpy.ops.wm.open_mainfile(filepath=str(path))
    scene = bpy.data.scenes.get('york Refinement') or bpy.context.scene
    bpy.context.window.scene = scene
    bpy.context.view_layer.update()
    graph = bpy.context.evaluated_depsgraph_get()
    graph.update()
    rows = {}
    for obj in scene.objects:
        if obj.type != 'MESH' or obj.hide_render or obj.get('asset_group') in args.exclude_group:
            continue
        evaluated = obj.evaluated_get(graph)
        mesh = evaluated.to_mesh()
        points = [list(evaluated.matrix_world @ v.co) for v in mesh.vertices]
        faces = [list(f.vertices) for f in mesh.polygons]
        rows[obj.name] = {'matrix': [list(row) for row in evaluated.matrix_world],
                          'points': points, 'faces': faces,
                          'source_node': obj.get('source_node')}
        evaluated.to_mesh_clear()
    return rows


baseline = OUT / 'grounding/york-grounded.blend'
before = snapshot(baseline)
after = snapshot(args.candidate)
differences = []
for name in sorted(set(before) | set(after)):
    if name not in before or name not in after:
        differences.append({'object': name, 'reason': 'added or removed visible context'})
        continue
    a, b = before[name], after[name]
    if len(a['points']) != len(b['points']) or a['faces'] != b['faces']:
        differences.append({'object': name, 'reason': 'evaluated topology changed'})
        continue
    error = max((abs(x-y) for p,q in zip(a['points'],b['points']) for x,y in zip(p,q)), default=0)
    matrix_error = max(abs(x-y) for p,q in zip(a['matrix'],b['matrix']) for x,y in zip(p,q))
    if error > .0001 or matrix_error > .0001:
        differences.append({'object': name, 'world_vertex_max_error': error,
                            'matrix_max_error': matrix_error})
report = {'baseline': str(baseline), 'candidate': str(args.candidate),
          'baseline_sha256': hashlib.sha256(baseline.read_bytes()).hexdigest(),
          'candidate_sha256': hashlib.sha256(args.candidate.read_bytes()).hexdigest(),
          'scope': 'Evaluated scene meshes with their own render flag enabled, outside explicitly changed groups. Includes collection-hidden reference meshes. Both saved scenes reopened and dependency graph updated before measurement.',
          'excluded_groups': args.exclude_group, 'baseline_meshes': len(before),
          'candidate_meshes': len(after), 'differences': differences,
          'preserved': not differences}
args.output.parent.mkdir(parents=True, exist_ok=True)
args.output.write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps({'preserved': not differences, 'differences': len(differences)}))
if differences:
    raise ValueError('Evaluated context differs; inspect audit before review handoff')
