"""Save a bounded phase animation without editing reviewed winch geometry."""
import ast
import hashlib
import json
import math
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
WORK = ROOT / 'level-editor/work/york-refinement/restart2'
BASE = WORK / 'winch-supported-hardware-v1'
OUT = WORK / 'winch-supported-motion-v1'
PLAN = WORK / 'winch-supported-motion-plan-v1.json'
CHECK = '--check-only' in sys.argv
if OUT.exists() and not CHECK:
    raise FileExistsError(OUT)


def budget():
    used = sum(p.stat().st_size for folder in (BASE, OUT, WORK / 'winch-guided-entry-candidate-v1')
               for p in folder.rglob('*') if p.is_file())
    assert used < 20 * 1024**2
    assert shutil.disk_usage(ROOT).free > 8 * 1024**3 + 20 * 1024**2 - used


if not CHECK:
    budget()
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
import numpy as np
from mathutils import Vector, Matrix
sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
from refinement_workspace import _geometry

plan = json.loads(PLAN.read_text())
assert hashlib.sha256((BASE / 'model.blend').read_bytes()).hexdigest() == plan['source_model_sha256']
s, c = math.sin(math.radians(35)), math.cos(math.radians(35))
up, screen_side, back = Vector((0, 0, 1)), Vector((1, 0, 0)), Vector((0, -c, s))
spacing = 3.5 / c
for name in ('restart13_winch_return_study.py', 'restart14_winch_guided_entry.py'):
    recipe = Path(__file__).with_name(name)
    exec(compile(ast.Module(body=[n for n in ast.parse(recipe.read_text()).body if isinstance(n, ast.FunctionDef)], type_ignores=[]), str(recipe), 'exec'))
center = world(2410, 1064, 104)
axis = (center - world(2402, 1050, 104)).normalized()
radial = Vector((-axis.y, axis.x, 0))
pose, params, path = guided_route(count=76)
bpy.ops.wm.open_mainfile(filepath=str(BASE / 'model.blend'))
scene = bpy.context.scene
links = sorted((o for o in scene.objects if o.name.startswith('Guided chain link')), key=lambda o: o.name)
assert len(links) == 76
protected = [o for o in scene.objects if o not in links]
guards = []
for row in plan['rows']:
    scene.frame_set(row['tick'])
    bpy.context.view_layer.update()
    guards.append({o.name: _geometry(o, protect_appearance=True) for o in protected})
scene.frame_set(88)
bpy.context.view_layer.update()
final_link_guard = {o.name: _geometry(o, protect_appearance=True) for o in links}
for row in plan['rows']:
    for i, obj in enumerate(links):
        point, rotation = pose(i * spacing + row['phase_native_pixels'] / c, i)
        obj.location, obj.rotation_euler = point, rotation.to_euler()
        obj.keyframe_insert('location', frame=row['tick'])
        obj.keyframe_insert('rotation_euler', frame=row['tick'])
for obj in links:
    obj.keyframe_insert('location', frame=90)
    obj.keyframe_insert('rotation_euler', frame=90)
    for layer in obj.animation_data.action.layers:
        for strip in layer.strips:
            bag = strip.channelbag(obj.animation_data.action_slot)
            for curve in bag.fcurves:
                for key in curve.keyframe_points:
                    key.interpolation = 'CONSTANT'
for index, row in enumerate(plan['rows']):
    scene.frame_set(row['tick'])
    bpy.context.view_layer.update()
    assert guards[index] == {o.name: _geometry(o, protect_appearance=True) for o in protected}
scene.frame_set(88)
bpy.context.view_layer.update()
assert final_link_guard == {o.name: _geometry(o, protect_appearance=True) for o in links}
if CHECK:
    print(json.dumps({'status': 'Read-only animation construction check passed; no model saved',
                      'protected_objects_all45': len(protected),
                      'final_links_exact': len(links), 'phase_count': len(plan['rows'])}))
    sys.exit(0)
scene.frame_start, scene.frame_end = 0, 90
scene.render.fps = 25
budget()
OUT.mkdir()
bpy.context.preferences.filepaths.save_version = 0
bpy.ops.wm.save_as_mainfile(filepath=str(OUT / 'model.blend'), compress=True)
assert (OUT / 'model.blend').stat().st_size < 1024**2
budget()
record = {'status': 'Private animation candidate, saved pose/contact and visual reviews pending',
          'model_sha256': hashlib.sha256((OUT / 'model.blend').read_bytes()).hexdigest(),
          'source_model_sha256': plan['source_model_sha256'],
          'plan_sha256': hashlib.sha256(PLAN.read_bytes()).hexdigest(),
          'outside_chain_geometry_appearance_exact_all45': len(protected),
          'final_material_link_geometry_appearance_exact': True,
          'rows': plan['rows'], 'duration_ticks': 90, 'tick_rate_hz': 25,
          'interpolation': 'CONSTANT', 'limitations': plan['limitations'] +
          ['Native transparent initial state, sound360 and final-state runtime integration remain separate.']}
(OUT / 'motion.json').write_text(json.dumps(record, indent=2) + '\n')
print(json.dumps({'model_bytes': (OUT / 'model.blend').stat().st_size, 'model_sha256': record['model_sha256']}))
