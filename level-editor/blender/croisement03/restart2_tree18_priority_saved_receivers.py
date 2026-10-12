"""Read-only joint wood/bank receivers for the two reviewed Tree18 source interiors."""
import json
import math
import sys
from pathlib import Path
import bpy
import numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
sys.path.insert(0, str(Path(__file__).resolve().parent))
import restart2_tree18_union_saved as union
B = union.B


def main():
    folder = B / 'tree18-priority-source-proposals-v1'
    proofpath = folder / 'receipt.json'
    proof = json.loads(proofpath.read_text())
    rootpath = folder / 'root-source-review.json'
    root = json.loads(rootpath.read_text())
    assert root['status'] == 'ROOT_SOURCE_ROLES_ACCEPTED_FOR_PRIVATE_RECEIVER_VALIDATION'
    assert root['receipt_sha256'] == union.sha(proofpath)
    pins = {str(proofpath): union.sha(proofpath), str(rootpath): union.sha(rootpath)}
    for name, digest in proof['pins'].items():
        assert union.sha(Path(name)) == digest, name
        pins[name] = digest
    for name, digest in proof['output_pins'].items():
        assert union.sha(folder / name) == digest, name
        pins[str(folder / name)] = digest
    models = [B / 'tree18-united-wood-saved-v1/worker.blend', B / 'bank-coupled-left-full-source-v3/worker.blend']
    for model in models:
        receipt = model.parent / ('receipt.json' if 'tree18' in model.parent.name else 'projection.json')
        assert union.sha(model) == json.loads(receipt.read_text())['model_sha256']
        pins[str(model)] = union.sha(model)
    out = B / 'tree18-priority-saved-receivers-v1'
    assert not out.exists()
    union.guard.resources(); union.guard.acquire(slots=2)
    try:
        union.guard.resources()
        rows = []
        for model in models:
            bpy.ops.wm.open_mainfile(filepath=str(model)); bpy.context.view_layer.update()
            for obj in bpy.context.scene.objects:
                if obj.type != 'MESH':
                    continue
                obj.data.calc_loop_triangles()
                triangles = [tuple(t.vertices) for t in obj.data.loop_triangles]
                faces = [t.polygon_index for t in obj.data.loop_triangles]
                vertices = [obj.matrix_world @ v.co for v in obj.data.vertices]
                rows.append((obj.name, BVHTree.FromPolygons(vertices, triangles, all_triangles=True), faces))
        source = np.array(Image.open(B.parent / 'baseline/covered.png').convert('RGBA'))
        inventory = json.loads((B.parent / 'source-inventory.json').read_text())
        animation = next(a for a in inventory['animations'] if a['index'] == 4)
        assert animation['sprite']['position_x'] == 209 and animation['sprite']['position_y'] == 162
        frames = []
        for row in animation['frames']:
            frame = np.array(Image.open(Path(row['image'])).convert('RGBA'))
            raw = np.array(Image.open(Path(row['source'])).convert('RGBA'))
            assert union.sha(Path(row['source'])) == row['sha256']
            raw[np.all(raw[:, :, :3] == [0, 251, 0], axis=2)] = 0
            assert np.array_equal(raw, frame)
            x, y, width, height = row['bbox']
            assert frame.shape[:2] == (height, width)
            frames.append((frame, x, y))
        assert len(frames) == 14
        regions = []
        for region, expected_count in zip(proof['regions'], [650, 31]):
            points = region['exact_source_pixels_xy']; assert len(points) == expected_count
            mask = np.array(Image.open(folder / (region['id'] + '-unassigned-interior.png'))) > 0
            assert {(int(x), int(y)) for y, x in np.argwhere(mask)} == {tuple(p) for p in points}
            rgba = np.array(Image.open(folder / (region['id'] + '-unassigned-source-rgba.png')).convert('RGBA'))
            assert np.array_equal(rgba[mask], source[mask])
            hits = []; failures = []
            for x, y in points:
                for i, (frame, frame_x, frame_y) in enumerate(frames):
                    xx, yy = x - frame_x, y - frame_y
                    assert not (0 <= yy < frame.shape[0] and 0 <= xx < frame.shape[1] and frame[yy, xx, 3]), ('native priority', i, x, y)
                origin = Vector((x + .5, -(y + .5) / union.S, 0)) + union.RAY * 5000
                candidates = []
                for name, tree, faces in rows:
                    hit, normal, triangle, distance = tree.ray_cast(origin, -union.RAY)
                    if hit is not None:
                        candidates.append({'object': name, 'face': faces[triangle], 'distance': distance, 'world': list(hit)})
                candidates.sort(key=lambda r: r['distance'])
                first = candidates[0] if candidates else None
                expected = first is not None and (first['object'] == 'Provisional Tree18 united wood' if expected_count == 650 else first['object'].startswith('Candidate bank '))
                record = {'source_xy': [x, y], 'source_rgba': source[y, x].tolist(), 'first': first, 'expected_class_hit': expected}
                hits.append(record)
                if not expected: failures.append(record)
            regions.append({'id': region['id'], 'pixels': len(points), 'hits': hits, 'failures': failures})
        assert all(union.sha(Path(p)) == digest for p, digest in pins.items())
        report = {'status': 'PASS_JOINT_SAVED_RECEIVERS_SOURCE_RGBA' if not any(r['failures'] for r in regions) else 'HOLD_RECEIVER_CLASS_MISMATCH', 'pins': pins, 'regions': regions, 'native_frame_boxes': [row['bbox'] for row in animation['frames']],
                  'limits': ['Read-only geometry first hits, separate wood and stone classes; no source assignment or texture bake performed.', 'All14 exact native source alpha excluded using each extracted frame bbox, including per-frame crop offsets from sprite209,162.', 'Static foliage not yet reconstructed or approved; this is not complete neighboring foliage/contact proof.', 'No geometry, material, native source pixel or canonical file changes.']}
        out.mkdir(); (out / 'receipt.json').write_text(json.dumps(report, indent=2) + '\n')
        print(report['status'], [(r['id'], len(r['failures'])) for r in regions])
    finally:
        union.guard.release()


if __name__ == '__main__':
    main()
