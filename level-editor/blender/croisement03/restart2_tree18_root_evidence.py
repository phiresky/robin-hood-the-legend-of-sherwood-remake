"""Separate lower Tree18 bark evidence from ivy and unmeasured root depth."""
import argparse
import hashlib
import json
import math
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[3]
BASE = ROOT / 'level-editor/work/croisement03-refinement'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    source_path = BASE / 'baseline/covered.png'
    level_path = BASE / 'baseline/Croisement03.rhp.json'
    alpha_path = BASE / 'restart2/tree18-source-context-v1/native-alpha-union.png'
    audit_path = BASE / 'restart2/tree18-cpu-depth-v2/depth-audit.json'
    source = Image.open(source_path).convert('RGB')
    level = json.loads(level_path.read_text())
    alpha = np.array(Image.open(alpha_path)) > 0
    prior = json.loads(audit_path.read_text())
    box = (370, 400, 490, 555)
    source.crop(box).resize((720, 930), Image.Resampling.NEAREST).save(args.output / 'root-original-6x.png')
    marked = source.copy()
    draw = ImageDraw.Draw(marked)
    regions = [
        {'id': 'A', 'box_xyxy': [432, 430, 446, 449], 'role': 'Definite exposed ridged bark interior',
         'confidence': 'high visual material identification; no outer-boundary claim', 'color': '#00ffff'},
        {'id': 'B', 'box_xyxy': [433, 469, 443, 490], 'role': 'Lower definite brown bark interior',
         'confidence': 'high visual material identification; ivy crosses nearby margins', 'color': '#00ffff'},
        {'id': 'C', 'box_xyxy': [401, 444, 420, 486], 'role': 'Green ivy, dark margins and mixed coverage',
         'confidence': 'mixed material; do not project as a solid bark strip', 'color': '#55ff55'},
        {'id': 'D', 'box_xyxy': [447, 465, 465, 508], 'role': 'Right lower edge with ivy and uncertain wood boundary',
         'confidence': 'outer silhouette unmeasured', 'color': '#ffaa33'},
        {'id': 'E', 'box_xyxy': [418, 494, 455, 522], 'role': 'Dark lower continuation, ivy and hidden contact',
         'confidence': 'actual trunk/ground intersection not visible', 'color': '#ff55ff'},
        {'id': 'F', 'box_xyxy': [379, 496, 415, 536], 'role': 'Left diagonal root-like form merging into ground',
         'confidence': 'possible root; depth and connection unverified', 'color': '#ff55ff'},
    ]
    for row in regions:
        draw.rectangle(row['box_xyxy'], outline=row['color'], width=1)
        draw.text((row['box_xyxy'][0], row['box_xyxy'][1] - 11), row['id'], fill=row['color'])
    draw.line((370, 462, 490, 462), fill='#ffff00', width=1)
    marked.crop(box).resize((720, 930), Image.Resampling.NEAREST).save(args.output / 'root-annotated-6x.png')
    rows = np.argwhere(alpha[:, 410:461])
    old = np.array(prior['old_occluded_pixels_xy'])
    nearby_lines = [e for e in level['elevation_lines']
                    if any(300 <= p[0] <= 550 and 400 <= p[1] <= 580 for p in [e['point_a'], e['point_b']])]
    obstacle_bounds = {}
    for i in [33, 34]:
        points = level['sight_obstacles'][i]['points']
        obstacle_bounds[str(i)] = {
            'source_metadata': level['sight_obstacles'][i],
            'native_x_range': [min(p['x'] for p in points), max(p['x'] for p in points)],
            'native_y_range': [min(p['y'] for p in points), max(p['y'] for p in points)],
            'meaning': 'Gameplay footprint/elevation context, not a measured visible bark or root contact boundary.'}
    cosine = math.cos(math.radians(35))
    report = {
        'status': 'HOLD_ROOT_DEPTH_AND_CANOPY_ATTACHMENT_REQUIRES_REDESIGN',
        'pins': {str(p): sha(p) for p in [source_path, level_path, alpha_path, audit_path]},
        'regions': regions,
        'visual_findings': [
            'The proposed full section x418..460 at y474 mixes bark, shadow and ivy; its endpoints are not locked measurements.',
            'Nevertheless y474 is not wholly ivy: central brown ridged bark persists through approximately y490.',
            'Exact lower bark extent and root/ground intersection below about y498 are obscured.',
            'The left diagonal root-like form is not enough evidence to invent a wide horizontal foot.',
            'All44 old source endpoints remain approximate proposals; none is promoted to a material mask or mandatory outer silhouette.'],
        'native_leaf_evidence': {
            'trunk_x_range_inclusive': [410, 460],
            'maximum_alpha_y_all14': int(rows[:, 0].max()),
            'alpha_pixels_at_y430_or_lower': int((rows[:, 0] >= 430).sum()),
            'old_occluded_pixels_bounds_xy': [old.min(0).tolist(), old.max(0).tolist()],
            'old_occluded_pixels_at_y430_or_lower': int((old[:, 1] >= 430).sum()),
            'conclusion': 'Recessing wood sections433/454/474 was not required by any native alpha pixel. The old CPU rayPASS cannot approve its distorted lower anatomy.'},
        'depth_conflict': {
            'native_plane_ground_source_y': 462,
            'formula': 'source_y = -world_y*sin35 - world_z*cos35; behind fixed native plane implies world_z <= (462-source_y)/cos35.',
            'maximum_z_behind_plane_at_y474': (462 - 474) / cosine,
            'maximum_z_behind_plane_at_y490': (462 - 490) / cosine,
            'conclusion': 'Visible lower bark cannot simultaneously stay at its source position, remain above z0 and lie behind this fixed plane. That plane is not physical root-depth evidence.'},
        'presentation_gap': {
            'authored_display_polyline': [[176, 567], [695, 576]],
            'display_rule': 'Animated effects are ordered relative to other elements using the authored display polyline and the other element position. This is distinct from a physical sprite card plane.',
            'preview_placement': 'scenery-layer.ts places the preview card using sprite position plus animation centre and elevation, giving the diagnostic source-ground intercept462.',
            'conclusion': 'The preview plane is not a measured depth constraint on Tree18 wood. Correct presentation parity independently; do not bend the trunk to satisfy this diagnostic plane.',
            'retained_shape_baseline': str(BASE / 'restart2/tree18-cpu-wood-v2/geometry.json'),
            'rejected_depth_shape': str(BASE / 'restart2/tree18-cpu-depth-v2/geometry.json'),
            'shared_runtime_edits': False},
        'terrain_metadata': {
            'tree_obstacles': obstacle_bounds,
            'nearby_elevation_lines': nearby_lines,
            'assessment': 'Nearby authored bank transition ends at x364,y477, left of this trunk. No listed transition directly establishes the contact at x430..450,y498..522. Native obstacle33/34 bottoms are zero, but their shared mean y~500 is a placement hypothesis, not measured root geometry.'},
        'possible_next_scope_without_mutation': [
            'Recover a coherent lower trunk around the definite bark interiors while allowing uncertain ivy-covered edges to change; do not retain the horizontal foot merely to protect old proposals.',
            'Keep root continuation narrow and connected beneath ivy; determine its ground contact using saved local terrain and bank context, not the gameplay footprint mean alone.',
            'Apply leaf-priority constraints only on exact native-alpha pixels and test all14 phases; no lower-trunk recession is justified by absent leaves.',
            'Retain CPU-wood-v2 natural wood as the provisional shape baseline. The fixed preview card depth must not force either lower or upper wood deformation.',
            'Resolve authored display-polyline ordering versus physical preview card placement independently, preserving single native-leaf owner and exact RGBA; require bank/neighbour and allphase saved coverage review.',
            'Do not assign bark or ivy source RGB until their material boundaries and exact phase priority are reviewed.'],
        'no_geometry_changes': True,
        'no_source_assignments': True,
        'heavy_execution': 'NONE'}
    (args.output / 'receipt.json').write_text(json.dumps(report, indent=2) + '\n')
    print(report['native_leaf_evidence'])


if __name__ == '__main__':
    main()
