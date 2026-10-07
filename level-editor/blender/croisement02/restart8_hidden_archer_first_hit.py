"""Read-only alpha-aware attribution of bounded hidden-archer source changes."""
import sys
import json
from pathlib import Path
from collections import Counter
import bpy
import numpy as np
from PIL import Image
from mathutils import Vector

HERE = Path(__file__).resolve().parent
sys.path[:0] = [str(HERE), str(HERE.parents[1] / 'refinement'), str(HERE.parents[1] / 'refinement/blender')]
from catalog import OUT
from evidence_io import sha, write_json
from tree_geometry import SIN, COS, RAY
from refinement_review import _tree
from render_slots import acquire, release


def main():
    source = OUT / 'restart2-textures/batch10-linked-static-v1/scene.blend'
    expected = '493eb8afa1f3e60f433ee2faa5e018d65552292fe5e2dfb63e96a5eb32acfd68'
    assert sha(source) == expected
    root = OUT / 'restart8-hidden-archer-receiver-audit-v1'
    current = '--current' in sys.argv
    output = root / ('current-first-hit-v1' if current else 'baseline-first-hit-v1')
    output.mkdir(exist_ok=False)
    report = json.loads((root / 'report.json').read_text())
    bpy.ops.wm.open_mainfile(filepath=str(source))
    scene = bpy.data.scenes['Croisement02 Refinement']
    bpy.context.window.scene = scene
    bpy.context.view_layer.update()
    all_objects = [o for o in bpy.data.collections['Croisement02 Working'].all_objects
                   if o.type == 'MESH' and not o.hide_render]
    replacements = []
    if current:
        for number, batch in [(18, 16), (19, 15), (24, 16), (25, 15)]:
            asset = f'croisement02-tree-{number:02d}'
            folder = OUT / f'restart3-review-batches/batch-v{batch}'
            approval = json.loads((folder / 'user-approval.json').read_text())
            assert sha(folder / 'evidence.json') == approval['evidence_sha256']
            member = next(m for c in approval['cards'] for m in c['members'] if m['asset_id'] == asset)
            assert sha(member['model']) == member['model_sha256']
            prior = set(bpy.data.objects)
            with bpy.data.libraries.load(member['model'], link=False) as (src, dst):
                dst.objects = src.objects
            added = [o for o in bpy.data.objects if o not in prior and o.type == 'MESH' and o.get('asset_group') == asset]
            assert added, asset
            for obj in added:
                scene.collection.objects.link(obj)
            all_objects = [o for o in all_objects if o.get('asset_group') != asset] + added
            replacements.append(dict(asset=asset, model=member['model'], model_sha256=member['model_sha256'],
                                     approval_sha256=sha(folder / 'user-approval.json'), objects=[o.name for o in added]))
        bpy.context.view_layer.update()
    records = []
    for row in report['profiles']:
        number = int(row['profile'][-2:])
        if number not in (3, 4):
            continue
        x0, y0, x1, y1 = row['crop']
        objects = []
        for obj in all_objects:
            points = np.array([obj.matrix_world @ Vector(p) for p in obj.bound_box])
            projected = np.column_stack((points[:, 0], -points[:, 1]*SIN-points[:, 2]*COS))
            lo, hi = projected.min(0), projected.max(0)
            if hi[0] >= x0 and lo[0] <= x1 and hi[1] >= y0 and lo[1] <= y1:
                objects.append(obj)
        tree, owners, _ = _tree(objects)
        changed = np.array(Image.open(root / f'{number:02d}-changed-mask.png').convert('L')) > 0
        counts, samples = Counter(), []
        for y, x in np.argwhere(changed):
            gx, gy = int(x+x0), int(y+y0)
            origin = Vector((gx+.5, -(gy+.5)/SIN, 0))+RAY*6000
            point, normal, index, distance = tree.ray_cast(origin, -RAY)
            obj = owners[index] if point is not None else None
            asset = (obj.get('asset_group') or obj.get('source_node') or obj.name) if obj else '<none>'
            counts[asset] += 1
            samples.append(dict(pixel=[gx, gy], asset=asset, object=obj.name if obj else None,
                                world=list(point) if point is not None else None,
                                source_node=obj.get('source_node') if obj else None))
        records.append(dict(profile=row['profile'], candidate_objects=[o.name for o in objects],
                            changed_centers=int(changed.sum()), first_hits=dict(counts), samples=samples))
        print(row['profile'], dict(counts), flush=True)
    assert sha(source) == expected
    write_json(output / 'report.json', dict(
        status='Measured scoped approved replacements; physical state endpoints still absent' if current else 'Measured frozen baseline, not current endpoint completion', source=str(source),
        source_sha256=expected, method='Native center rays; shared material/UV alpha-aware one-sided BVH',
        replacements=replacements, profiles=records, limits=[
            'Scoped tree replacements are recorded above; other baseline receivers may predate later ground/bank derivatives.',
            'Receiver identity informs construction; unchanged sprite centers and full neighbor support still require endpoint audits.',
            'No source mask or existing model has been changed; no physical endpoint state is yet constructed.']))


if __name__ == '__main__':
    acquire()
    try:
        main()
    finally:
        release()
