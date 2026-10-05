"""Record the unchanged hall's game-space surfaces for source-led refinement."""
import hashlib
import json
import math
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'level-editor/work/york-refinement'
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy

source = OUT / 'geometry-pass-01/assets/york-castle-great-hall/model.blend'
destination = OUT / 'restart2/hall-baseline-surfaces.json'
if destination.exists():
    raise FileExistsError(destination)
bpy.ops.wm.open_mainfile(filepath=str(source))
sine, cosine = math.sin(math.radians(35)), math.cos(math.radians(35))
rows = []
for obj in bpy.data.collections['york Working'].all_objects:
    if obj.type != 'MESH' or obj.hide_render or obj.get('asset_group') != 'york-castle-great-hall':
        continue
    points = [obj.matrix_world @ v.co for v in obj.data.vertices]
    rows.append({'object': obj.name, 'source_node': obj['source_node'],
                 'component': obj.get('projection_component'),
                 'game_vertices': [[v.x, -v.y*sine, v.z*cosine] for v in points],
                 'source_vertices': [[v.x, -v.y*sine-v.z*cosine] for v in points],
                 'faces': [list(p.vertices) for p in obj.data.polygons]})
if len(rows) != 11:
    raise ValueError(f'Expected eleven hall components, found {len(rows)}')
destination.write_text(json.dumps({'source':str(source),'sha256':hashlib.sha256(source.read_bytes()).hexdigest(),
                                 'status':'Unchanged diagnostic; not a geometry approval','objects':rows},indent=2)+'\n')
print(destination)
