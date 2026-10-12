"""CPU native-alpha first-hit audit and ray-preserving Tree18 depth proposal."""
import argparse
import copy
import hashlib
import json
import math
from pathlib import Path

import numpy as np
from PIL import Image

S = math.sin(math.radians(35))
C = math.cos(math.radians(35))
RAY = np.array([0., -C, S])
PLANE_Y = -(162 + 200 + 100) / S


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def projection(v):
    return np.column_stack((v[:, 0], -v[:, 1] * S - v[:, 2] * C))


def audit(parts, pixels):
    xy = pixels[:, ::-1] + .5
    native_z = (-xy[:, 1] - PLANE_Y * S) / C
    native_depth = -PLANE_Y * C + native_z * S
    nearest = np.full(len(xy), -np.inf)
    owners = np.full(len(xy), -1)
    for owner, part in enumerate(parts):
        v = np.array(part['vertices'])
        screen = projection(v)
        for face in part['faces']:
            tri = screen[face]
            lo, hi = tri.min(0), tri.max(0)
            selection = np.flatnonzero(np.all((xy >= lo - 1e-8) & (xy <= hi + 1e-8), axis=1))
            if not len(selection):
                continue
            a, b = tri[1] - tri[0], tri[2] - tri[0]
            determinant = a[0] * b[1] - a[1] * b[0]
            if abs(determinant) < 1e-10:
                continue
            q = xy[selection] - tri[0]
            u = (q[:, 0] * b[1] - q[:, 1] * b[0]) / determinant
            w = (a[0] * q[:, 1] - a[1] * q[:, 0]) / determinant
            inside = (u >= -1e-9) & (w >= -1e-9) & (u + w <= 1 + 1e-9)
            selection, u, w = selection[inside], u[inside], w[inside]
            hit = v[face[0]] + u[:, None] * (v[face[1]] - v[face[0]]) + w[:, None] * (v[face[2]] - v[face[0]])
            depth = hit @ RAY
            wins = depth > nearest[selection]
            nearest[selection[wins]] = depth[wins]
            owners[selection[wins]] = owner
    return nearest > native_depth + 1e-5, nearest - native_depth, owners


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--geometry', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    data = json.loads(args.geometry.read_text())
    source_path = Path(data['source'])
    assert sha(source_path) == data['source_sha256']
    source = json.loads(source_path.read_text())
    image_path = next(Path(p) for p in source['pins'] if p.endswith('/baseline/covered.png'))
    size = Image.open(image_path).size
    masks = []
    for frame in source['frames']:
        path = Path(frame['normalized'])
        assert sha(path) == frame['normalized_sha256']
        canvas = Image.new('RGBA', size)
        canvas.paste(Image.open(path).convert('RGBA'), tuple(frame['bbox_xywh'][:2]))
        masks.append(np.array(canvas)[:, :, 3] > 0)
    pixels = np.argwhere(np.any(masks, axis=0))
    memberships = np.array([m[pixels[:, 0], pixels[:, 1]] for m in masks])
    before, before_depth, before_owner = audit(data['parts'], pixels)
    changed = copy.deepcopy(data)
    provenance = []
    for part in changed['parts']:
        original = np.array(part['vertices'])
        revised = original.copy()
        n = part['ring_sides']
        ring_count = len(part['ring_roles'])
        shifts = []
        for i, role in enumerate(part['ring_roles']):
            ring = original[i * n:(i + 1) * n]
            required = max(0., (PLANE_Y + .1 - ring[:, 1].min()) / C)
            ground_limit = max(0., ring[:, 2].min() / S)
            amount = 0. if role == 'inferred_ground_scaffold' else min(required, ground_limit)
            revised[i * n:(i + 1) * n] -= amount * RAY
            shifts.append({'ring': i, 'role': role, 'recession_along_negative_native_ray': amount,
                           'required_to_clear_whole_ring': required, 'nonnegative_z_limit': ground_limit})
        revised[n * ring_count] -= shifts[0]['recession_along_negative_native_ray'] * RAY
        revised[n * ring_count + 1] -= shifts[-1]['recession_along_negative_native_ray'] * RAY
        error = float(np.max(np.abs(projection(original) - projection(revised))))
        assert error < 1e-9
        assert revised[:, 2].min() >= -1e-8
        for i in range(ring_count):
            old = original[i * n:(i + 1) * n]
            new = revised[i * n:(i + 1) * n]
            assert np.max(np.abs((old - old[0]) - (new - new[0]))) < 1e-9
        part['vertices'] = revised.tolist()
        part['bounds_world'] = [revised.min(0).tolist(), revised.max(0).tolist()]
        volume = sum(np.dot(revised[a], np.cross(revised[b], revised[c])) / 6 for a, b, c in part['faces'])
        assert volume > 0
        part['volume'] = float(volume)
        areas = [np.linalg.norm(np.cross(revised[b] - revised[a], revised[c] - revised[a])) / 2
                 for a, b, c in part['faces']]
        assert min(areas) > 1e-8
        part['minimum_triangle_area'] = float(min(areas))
        provenance.append({'track': part['track'], 'original_world_vertices': original.tolist(),
                           'ring_shifts': shifts, 'source_projection_max_error': error,
                           'all_ring_shapes_unchanged': True})
    after, after_depth, after_owner = audit(changed['parts'], pixels)
    report = {'status': 'PASS_CPU_ALL14_NATIVE_ALPHA_PRIORITY' if not after.any() else 'HOLD_RESIDUAL_NATIVE_ALPHA_OCCLUSION',
              'input_geometry': str(args.geometry.resolve()), 'input_sha256': sha(args.geometry),
              'native_plane_y': PLANE_Y, 'native_plane_derivation': '-(sprite_y162 + frame_center_y200 + elevation100)/sin35',
              'pixel_sampling': 'Exact alpha pixel centres x+0.5,y+0.5; projected triangle barycentrics and native-ray depth.',
              'union_alpha_pixels': len(pixels), 'before_union_occluded': int(before.sum()),
              'after_union_occluded': int(after.sum()),
              'phases': [{'phase': i, 'alpha_pixels': int(m.sum()),
                          'before_occluded': int((memberships[i] & before).sum()),
                          'after_occluded': int((memberships[i] & after).sum())} for i, m in enumerate(masks)],
              'remaining_pixels_xy': pixels[after][:, ::-1].tolist(),
              'old_occluded_pixels_xy': pixels[before][:, ::-1].tolist(),
              'provenance': provenance,
              'limits': ['Private inferred depth correction; no source assignment or native sprite changes.',
                         'Every original world point is retained in provenance and prior geometry stays untouched.',
                         'Ring translations preserve exact cross-section shape, round depth and source projection.',
                         'Root scaffold remains fixed z0; all vertices remain nonnegative z.',
                         'The ground-clamped lower transition leans in depth and has additional low points; its anatomy and terrain contact need oblique review.',
                         'Saved model, self-intersections, closed junction union and bank contacts remain unvalidated.']}
    args.output.mkdir(parents=True, exist_ok=False)
    changed['native_alpha_depth_audit'] = {k: v for k, v in report.items() if k not in ['provenance', 'old_occluded_pixels_xy', 'remaining_pixels_xy']}
    changed['status'] = report['status'] + '_PRIVATE_GEOMETRY_CONTACT_HOLD'
    changed.pop('native_wire_sha256', None)
    (args.output / 'geometry.json').write_text(json.dumps(changed, indent=2) + '\n')
    (args.output / 'depth-audit.json').write_text(json.dumps(report, indent=2) + '\n')
    print({k: report[k] for k in ['status', 'before_union_occluded', 'after_union_occluded', 'native_plane_y']})


if __name__ == '__main__':
    main()
