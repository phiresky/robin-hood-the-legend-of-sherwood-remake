"""Fair inferred upper wood while freezing authoritative rays and bank contacts.

Unlike the earlier depth-only trials, the unobserved silhouette may change.
Every face hit by a protected source pixel remains exact, and an independent
closest-hit check rejects newly occluding inferred surfaces.
"""
import hashlib
import json
from pathlib import Path
import shutil
import sys

import numpy as np
from PIL import Image
from scipy.sparse import coo_matrix, diags
from vtkmodules.vtkCommonCore import reference
from vtkmodules.vtkCommonDataModel import vtkGenericCell, vtkStaticCellLocator

from restart2_tree08_fork_kernel import poly
from restart2_tree08_local_fork import audit
from restart2_tree08_junction_proof import native_depth

R = Path(__file__).resolve().parents[3] / 'level-editor/work/croisement01-refinement/restart2'
orientation_guard = True
O = R / 'tree08-unified-source-fairing-cpu-v1'
assert not O.exists()
assert shutil.disk_usage(R).free >= 10 * 1024**3 + 16 * 1024**2
assert next(int(s.split()[1]) for s in Path('/proc/meminfo').read_text().splitlines() if s.startswith('MemAvailable:')) >= 6 * 1024**2
O.mkdir()
source = R / 'tree08-inferred-shell-cleanup-v3-pocket/candidate.npz'
data = np.load(source)
v, f = data['vertices'], data['faces']
s, c = np.sin(np.radians(35)), np.cos(np.radians(35))
ray, down = np.array([0., -c, s]), np.array([0., -s, -c])
native_y = v @ down
mask_path = R / 'tree08-semantic-source-v1/bark-core-proposal.png'
core = np.asarray(Image.open(mask_path)) > 0
pixels = np.argwhere(core)
positions = np.array([np.array([x + 331.5, 0, 0]) + down * (y + 11.5) for y, x in pixels])


def trace(vertices):
    locator = vtkStaticCellLocator()
    locator.SetDataSet(poly(vertices, f))
    locator.BuildLocator()
    cell = vtkGenericCell()
    faces, hits = [], []
    for point in positions:
        t, sub, face = reference(0.), reference(0), reference(0)
        hit, uvw = [0., 0., 0.], [0., 0., 0.]
        found = locator.IntersectWithLine(point + ray * 2000, point - ray * 2000, 1e-9, t, hit, uvw, sub, face, cell)
        assert found, 'Source-owned pixel lost'
        faces.append(int(face))
        hits.append(hit)
    return np.array(faces), np.array(hits)


source_faces, source_hits = trace(v)
source_pins = np.unique(f[np.unique(source_faces)])
edges = np.unique(np.sort(np.concatenate([f[:, [0, 1]], f[:, [1, 2]], f[:, [2, 0]]]), axis=1), axis=0)
row, col = np.r_[edges[:, 0], edges[:, 1]], np.r_[edges[:, 1], edges[:, 0]]
adjacency = coo_matrix((np.ones(len(row)), (row, col)), shape=(len(v), len(v))).tocsr()
degree = np.asarray(adjacency.sum(axis=1)).ravel()
average = diags(1 / degree) @ adjacency
pinned = native_y >= 315
pinned[source_pins] = True
pinned[np.unique(edges[np.linalg.norm(v[edges[:, 0]] - v[edges[:, 1]], axis=1) < .003])] = True
passes = []
original_triangles=v[f]
original_normals=np.cross(original_triangles[:,1]-original_triangles[:,0],original_triangles[:,2]-original_triangles[:,0])
original_area2=np.einsum('ij,ij->i',original_normals,original_normals)
orientation_passes=[]
origin = np.array([552., -672., 235.])
for iteration in range(8):
    vertices = v.copy()
    for step in range(40):
        for gain in [.4, -.405]:
            update = gain * (average @ vertices - vertices)
            vertices[~pinned] += update[~pinned]
            delta = vertices - v
            lengths = np.linalg.norm(delta, axis=1)
            over = lengths > 10.
            vertices[over] = v[over] + delta[over] * (10. / lengths[over])[:, None]
            vertices[pinned] = v[pinned]
    if orientation_guard:
        displacement=vertices-v;weights=np.ones(len(v));weights[pinned]=0
        for shrink in range(80):
            vertices=(v+displacement*weights[:,None]-origin).astype(np.float32).astype(float)+origin
            vertices[weights<1e-3]=v[weights<1e-3]
            triangles=vertices[f]
            normals=np.cross(triangles[:,1]-triangles[:,0],triangles[:,2]-triangles[:,0])
            ratio=np.einsum('ij,ij->i',normals,original_normals)/original_area2
            folded=ratio<.05
            if not folded.any():break
            weights[np.unique(f[folded])]*=.5
        assert not folded.any(), 'Unable to prevent local surface folds'
        orientation_passes.append(dict(iteration=iteration,steps=shrink,min_oriented_area_ratio=float(ratio.min()),damped_vertices=int((weights<1).sum())))
    else:
        vertices = (vertices - origin).astype(np.float32).astype(float) + origin
        vertices[pinned] = v[pinned]
    after_faces, after_hits = trace(vertices)
    error = np.linalg.norm(after_hits - source_hits, axis=1)
    bad = error > .0001
    passes.append(dict(iteration=iteration, moved_source_hits=int(bad.sum()), max_hit_error=float(error.max()), pinned_vertices=int(pinned.sum())))
    print(json.dumps(passes[-1]), flush=True)
    if not bad.any():
        break
    more = np.zeros(len(v), bool)
    more[np.unique(f[after_faces[bad]])] = True
    more[np.unique(f[more[f].any(axis=1)])] = True
    pinned |= more
assert not bad.any(), 'Unobserved fairing still occludes source-owned pixels'
assert np.array_equal(vertices[source_pins], v[source_pins])
assert np.array_equal(vertices[native_y >= 315], v[native_y >= 315])
topology = audit(vertices, f)
assert not any(topology[k] for k in ['nonmanifold_edges', 'winding_errors', 'zero_area'])
before_depth, after_depth = native_depth([(v, f)]), native_depth([(vertices, f)])
lost = np.isfinite(before_depth) & ~np.isfinite(after_depth)
gained = ~np.isfinite(before_depth) & np.isfinite(after_depth)
assert not (lost & core).any()
np.savez_compressed(O / 'candidate.npz', vertices=vertices, faces=f, before_vertices=v, source_faces=np.unique(source_faces))
report = dict(status='CPU_INFERRED_GEOMETRY_CANDIDATE_NOT_REVIEW_READY',
              candidate_sha256=hashlib.sha256((O / 'candidate.npz').read_bytes()).hexdigest(), parent_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),
              source_mask_sha256=hashlib.sha256(mask_path.read_bytes()).hexdigest(), source_core_pixels=int(core.sum()),
              source_face_vertices_exact=True, source_first_hit_max_error=float(error.max()), lower_root_positions_exact=True,
              changed_vertices=int(np.any(vertices != v, axis=1).sum()), max_displacement=float(np.linalg.norm(vertices-v, axis=1).max()),
              inferred_silhouette_lost=int(lost.sum()), inferred_silhouette_gained=int(gained.sum()), source_owned_lost=int((lost & core).sum()),
              topology=topology, source_occlusion_constraint_passes=passes, orientation_constraint_passes=orientation_passes,
              scope='Reshape only unobserved upper geometry; exact source-hit face geometry and lower bank contacts retained. New inferred silhouette requires geometry review. Source RGB/UV and strict intersections need saved/independent checks. No texture synthesis.')
(O / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps(report, indent=2), flush=True)
