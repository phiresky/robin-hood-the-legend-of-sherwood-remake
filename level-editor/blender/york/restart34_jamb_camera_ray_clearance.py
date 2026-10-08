"""Prepare a native-projection-preserving, inferred jamb back-depth correction."""
import hashlib
import json
import math
from pathlib import Path

import numpy as np
from PIL import Image
from shapely.geometry import Polygon
from shapely.ops import triangulate

ROOT = Path(__file__).resolve().parents[3]
W = ROOT / 'level-editor/work/york-refinement/restart2'
D = W / 'gate-saved-contact-audit-v1'
OUT = W / 'jamb-hidden-clearance-plan-v2'
assert not OUT.exists(), 'Keep prior planning evidence immutable'
audit_path = D / 'report.json'
audit = json.loads(audit_path.read_text())
old = np.array(audit['geometry_world']['jamb_vertices'])
faces = audit['geometry_world']['jamb_faces']
gate = np.array(audit['geometry_world']['gate_vertices'])
base = old[0]
normal = old[13] - base
normal /= np.linalg.norm(normal)
axis = np.array([-normal[1], normal[0], 0])
s = math.sin(math.radians(35))
c = math.cos(math.radians(35))
camera_back = np.array([0., -c, s])
depth = lambda p: (p - base) @ normal
clearance_depth = float(max(depth(gate)) + .05)
new = old.copy()
steps = (clearance_depth - depth(old[:13])) / (camera_back @ normal)
new[:13] += steps[:, None] * camera_back
assert np.array_equal(old[13:], new[13:])
assert np.max(np.abs(depth(new[:13]) - clearance_depth)) < 1e-10
assert min(depth(new[13:])) > clearance_depth

def project(v):
    return np.column_stack([v[:, 0], -v[:, 1] * s - v[:, 2] * c])

screen_error = float(np.max(np.abs(project(old) - project(new))))
assert screen_error < 1e-9
# Preserve triangulation and loop topology. The later save recipe must preserve UVs.
triangle_ids = []
for face in faces:
    if len(face) <= 4:
        triangle_ids.extend([(face[0], face[i], face[i + 1]) for i in range(1, len(face) - 1)])
    else:
        points = [(float((old[i] - base) @ axis), float(old[i, 2])) for i in face]
        polygon = Polygon(points)
        for tri in triangulate(polygon):
            if polygon.covers(tri.representative_point()):
                triangle_ids.append(tuple(face[points.index(tuple(p))] for p in list(tri.exterior.coords)[:3]))
mask_path = W / 'jamb-source-probe-v1/visible-jamb-domain.png'
coords = np.argwhere(np.array(Image.open(mask_path).convert('L')) > 0)
pixels = np.array([[int(x) + 2250, int(y) + 780] for y, x in coords])
assert len(pixels) == 660
origins = np.column_stack([pixels[:, 0] + .5, -(pixels[:, 1] + .5) / s, np.zeros(len(pixels))]) + camera_back * 10000
ray = -camera_back

def cast(vertices):
    nearest = np.full(len(pixels), np.inf)
    owner = np.full(len(pixels), -1, dtype=int)
    bary = np.full((len(pixels), 2), np.nan)
    for index, ids in enumerate(triangle_ids):
        a, b, d = vertices[list(ids)]
        e1, e2 = b - a, d - a
        h = np.cross(ray, e2)
        det = e1 @ h
        if abs(det) < 1e-10:
            continue
        q = origins - a
        u = q @ h / det
        cross = np.cross(q, e1)
        v = cross @ ray / det
        t = cross @ e2 / det
        hits = (u >= -1e-7) & (v >= -1e-7) & (u + v <= 1 + 1e-7) & (t >= 0) & (t < nearest - 1e-7)
        nearest[hits], owner[hits] = t[hits], index
        bary[hits] = np.column_stack([u[hits], v[hits]])
    return nearest, owner, bary

before_t, before_owner, before_bary = cast(old)
after_t, after_owner, after_bary = cast(new)
assert np.all(np.isfinite(before_t)) and np.all(np.isfinite(after_t))
assert np.array_equal(before_owner, after_owner), 'Source ray changed triangle ownership'
bary_error = float(np.max(np.abs(before_bary - after_bary)))
assert bary_error < 1e-7, 'UV interpolation changed in native view'
areas = [float(np.linalg.norm(np.cross(new[j] - new[i], new[k] - new[i]))) / 2 for i, j, k in triangle_ids]
assert min(areas) > .001
# This correction is globally separated from every vertically translated gate pose.
gap = float(min(depth(new)) - max(depth(gate)))
assert gap >= .05 - 1e-9
report = {
    'status': 'CPU_PROJECTION_AND_CLEARANCE_PASS_NOT_SAVED_OR_APPROVED',
    'audit_sha256': hashlib.sha256(audit_path.read_bytes()).hexdigest(),
    'saved_gate_source': audit['model'], 'saved_gate_sha256': audit['model_sha256'],
    'authoritative_domain': str(mask_path),
    'domain_sha256': hashlib.sha256(mask_path.read_bytes()).hexdigest(),
    'operation': 'Move only 13 back-plane jamb vertices along native camera rays to the separating plane; retain all 13 front vertices, face topology, UV loops, material slots, gate and motion.',
    'old_vertices_world': old.tolist(), 'proposed_vertices_world': new.tolist(),
    'faces': faces, 'triangle_indices_for_guard': triangle_ids,
    'normal_world': normal.tolist(), 'camera_back_world': camera_back.tolist(),
    'new_back_depth': clearance_depth, 'all_continuous_vertical_poses_min_gap': gap,
    'minimum_jamb_normal_thickness': float(min(depth(new[13:])) - clearance_depth),
    'maximum_back_shift_world': float(max(steps)),
    'back_shift_world_xyz_range': [np.min(new[:13] - old[:13], axis=0).tolist(), np.max(new[:13] - old[:13], axis=0).tolist()],
    'native_vertex_projection_max_error': screen_error,
    'native_source_pixels': len(pixels), 'same_hit_triangle_pixels': int(np.sum(before_owner == after_owner)),
    'native_barycentric_max_error': bary_error,
    'source_ray_depth_changed_pixels': int(np.sum(np.abs(before_t - after_t) > 1e-4)),
    'minimum_triangle_area': min(areas),
    'save_guards': [
        'Verify exact final approved textured jamb vertex/world topology matches this extracted jamb before applying; abort on discrepancy.',
        'Assert all 13 front vertices, all UV loops/materials/images and every unscoped object remain byte-identical.',
        'Verify saved/reopened native textured pixels against baseline, source silhouette, 16 review views, and actual gate/jamb contacts across all 45 poses.',
        'Keep exact native art playback and timing; frame 36 lift remains explicitly inferred within the recorded source uncertainty.',
    ],
    'approval_scope': 'New inferred jamb back profile/depth and proposed 45-pose mechanics; prior jamb geometry approval does not cover this correction.',
    'limitations': ['CPU triangulation/projection proof only; no changed model, render, texture synthesis, runtime or canonical publication.', 'The preserved source colors rely on unchanged UV loops/material data; this must also pass the saved-model pixel comparison.'],
}
OUT.mkdir()
(OUT / 'proposal.json').write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps({k: report[k] for k in ['status', 'native_source_pixels', 'same_hit_triangle_pixels', 'native_barycentric_max_error', 'all_continuous_vertical_poses_min_gap', 'back_shift_world_xyz_range']}))
