"""Fit inferred ray depth with per-triangle orientation constraints.

This private CPU study preserves every source projection and fixes the root
and extreme tips. It broadens inferred depth while smoothing section seams. It does not establish source ownership or approve a saved model.
"""
import hashlib
import json
from pathlib import Path
import numpy as np
from scipy.optimize import linprog
from scipy.spatial import cKDTree
from scipy.sparse import coo_matrix, eye, hstack, vstack
from continuous_wood_field import RAY, SIN, COS

ROOT = Path(__file__).resolve().parents[3]
BASE = ROOT / 'level-editor/work/active/croisement02/wood-sweep-cpu-review'


def main():
    source = BASE / 'physical-full-visible38-oct10-v6/preview-mesh.npz'
    output = BASE / 'physical-full-visible38-ray-fair-oct11-v1'
    if output.exists():
        raise FileExistsError(output)
    mesh = np.load(source)
    vertices, faces = mesh['vertices'], mesh['faces']
    triangles = vertices[faces]
    normals = np.cross(triangles[:, 1]-triangles[:, 0], triangles[:, 2]-triangles[:, 0])
    squared = np.sum(normals*normals, axis=1)
    count = len(vertices)
    vertex_normals = np.zeros_like(vertices)
    for i in range(3):
        np.add.at(vertex_normals, faces[:, i], normals)
    vertex_normals /= np.maximum(np.linalg.norm(vertex_normals, axis=1)[:, None], 1e-12)
    distance, near = cKDTree(vertices).query(vertices, k=64)
    weights = np.exp(-distance**2/(2*1.5**2))*np.maximum(0, np.einsum('ij,ikj->ik', vertex_normals, vertex_normals[near]))**8
    weights /= weights.sum(axis=1)[:, None]
    smoothing = coo_matrix((weights.ravel(), (np.repeat(np.arange(count), 64), near.ravel())), shape=(count, count)).tocsr()
    z = vertices[:, 2]
    weight = np.clip((z-5)/10, 0, 1)*np.clip((283-z)/8, 0, 1)
    weight = weight*weight*(3-2*weight)
    depth = vertices @ RAY
    ground = next(r['ground_y'] for r in json.loads((BASE.parent/'forest-v4-sources/manifest.json').read_text()) if r['mask']==38)
    thickening = .09*(vertices[:,1]+ground/SIN)/RAY[1]
    target = np.clip((smoothing @ (smoothing @ (smoothing @ depth))-depth+thickening)*weight, -3, 3)
    # Parallel ray offsets make each triangle normal affine in its three
    # offsets; constrain those exact normals instead of shrinking every move
    # to accommodate a single nearly tangent triangle.
    a = np.einsum('ij,ij->i', np.cross(RAY, triangles[:, 2]-triangles[:, 0]), normals)/squared
    b = np.einsum('ij,ij->i', np.cross(triangles[:, 1]-triangles[:, 0], RAY), normals)/squared
    coefficients = np.column_stack((-a-b, a, b))
    orientation = coo_matrix((coefficients.ravel(), (np.repeat(np.arange(len(faces)), 3), faces.ravel())), shape=(len(faces), count)).tocsr()
    identity = eye(count, format='csr')
    constraints = vstack([hstack([-orientation, coo_matrix((len(faces), count))]), hstack([identity, -identity]), hstack([-identity, -identity])], format='csr')
    rhs = np.r_[np.full(len(faces), .65), target, -target]
    bounds = [(-float(3*w), float(3*w)) for w in weight] + [(0, None)]*count
    result = linprog(np.r_[np.zeros(count), np.ones(count)], A_ub=constraints, b_ub=rhs, bounds=bounds, method='highs', options={'time_limit': 120})
    if not result.success:
        raise RuntimeError(result.message)
    delta = result.x[:count]
    factor = 1.
    while factor > 1e-5:
        candidate = vertices + (delta*factor)[:, None]*RAY
        tri = candidate.astype(np.float32)[faces]
        new_normals = np.cross(tri[:, 1]-tri[:, 0], tri[:, 2]-tri[:, 0])
        fraction = np.sum(new_normals*normals, axis=1)/squared
        area = np.linalg.norm(new_normals, axis=1)/2
        if fraction.min() > .1 and area.min() > 1e-9:
            break
        factor *= .5
    else:
        raise ValueError('Float32 orientation guard rejected fitted depths')
    fixed = (z <= 5) | (z >= 283)
    assert np.array_equal(candidate[fixed], vertices[fixed])
    projection = lambda p: np.column_stack((p[:, 0], -p[:, 1]*SIN-p[:, 2]*COS))
    drift = float(np.abs(projection(candidate)-projection(vertices)).max())
    assert drift < 1e-8
    output.mkdir()
    np.savez_compressed(output/'preview-mesh.npz', vertices=candidate, faces=faces)
    report = dict(status='CPU STUDY ONLY; no saved-model or visual approval', source=str(source), source_sha256=hashlib.sha256(source.read_bytes()).hexdigest(), maximum_displacement=float(np.abs(delta*factor).max()), moved_vertices_over_point1=int(np.count_nonzero(np.abs(delta*factor)>.1)), source_projection_drift=drift, minimum_float32_normal_fraction=float(fraction.min()), minimum_float32_triangle_area=float(area.min()), boundary_vertices_fixed=True, float32_step_factor=factor, solver_message=result.message, target_l1_error=float(np.abs(delta*factor-target).sum()))
    previous=json.loads((source.parent/'report.json').read_text())
    previous['ray_fairing']=report
    from physical_wood_sections_cpu import projection_coverage
    from prepare_wood_field_integration import source as load_source
    box=previous['source_box'];observed,_=load_source(38,box)
    previous['observed_source_ground']=projection_coverage(candidate,faces,observed,box[:2])
    if previous['observed_source_ground']['missing_body_centers']:raise ValueError('Fairing lost source coverage')
    (output/'report.json').write_text(json.dumps(previous, indent=2)+'\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
