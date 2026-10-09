"""Diagnose remaining tiny bark wraps using bounded paths across quad edges."""
import collections
import hashlib
import json
from pathlib import Path

import numpy as np
from scipy.optimize import minimize

R = Path(__file__).resolve().parents[3] / 'level-editor/work/croisement01-refinement/restart2/tree02-filtered-gap-probe-v3'
probe = json.loads((R / 'report.json').read_text())
plan = json.loads((R / 'wrapped-bark-donor-plan.json').read_text())
packet = probe['packets']['Tree02 upper stems']
vertices = {int(k): np.array(v) for k, v in packet['vertices_world'].items()}
triangles = collections.defaultdict(list)
for tri in packet['triangles']:
    triangles[tri['face']].append(tri)
quads = {f for f, ts in triangles.items() if len(ts) == 2 and len({v for t in ts for v in t['vertices']}) == 4}
adjacency = collections.defaultdict(set)
edges = {}
for e in packet['adjacency']:
    if len(e['faces']) == 2:
        a, b = e['faces']
        adjacency[a].add(b)
        adjacency[b].add(a)
        edges[frozenset([a, b])] = np.array([vertices[v] for v in e['vertices']])
remaining = []
for rejection in plan['rejected']:
    row = rejection['closest_excluded']
    first, last = row['target_face'], row['donor_face']
    assert first in quads and last in quads and row['donor_ownership'] == 2
    target = row['target']
    donor_points, target_points = [], []
    for tri in triangles[last]:
        weights = np.linalg.solve(np.vstack([(np.array(tri['uv']) * packet['atlas_size']).T, np.ones(3)]), np.r_[np.array(row['donor']) + .5, 1])
        if weights.min() >= -1e-6:
            donor_points.append(weights @ np.array([vertices[v] for v in tri['vertices']]))
    for witness in probe['witnesses']:
        for hit in witness['hits']:
            if hit['object'] != 'Tree02 upper stems' or hit['face'] != first or hit['atlas_texel'] != target:
                continue
            for tri in triangles[first]:
                weights = np.array(hit['weights'])
                if np.array_equal((weights @ (np.array(tri['uv']) * packet['atlas_size'])).astype(int), target):
                    target_points.append(weights @ np.array([vertices[v] for v in tri['vertices']]))
    paths, routes = [[first]], []
    for hop in range(5):
        routes.extend(p for p in paths if p[-1] == last)
        paths = [p + [n] for p in paths for n in adjacency[p[-1]] if n in quads and n not in p]
    solutions = []
    for path in routes:
        segments = np.array([edges[frozenset([a, b])] for a, b in zip(path, path[1:])])
        for a in target_points:
            for b in donor_points:
                def length(t):
                    points = np.vstack([a, segments[:, 0] + t[:, None] * (segments[:, 1] - segments[:, 0]), b])
                    return float(np.linalg.norm(np.diff(points, axis=0), axis=1).sum())
                result = minimize(length, np.full(len(segments), .5), bounds=[(0, 1)] * len(segments), method='SLSQP', options={'maxiter': 100, 'ftol': 1e-10})
                if result.success and length(result.x) <= 1.0:
                    solutions.append(dict(row, quad_path=path, shared_edge_parameters=result.x.tolist(), connected_quad_route_length=length(result.x), inference='generated bark continuation around connected subpixel sleeve; inferred RGB only'))
    if solutions:
        plan['plans'].append(min(solutions, key=lambda s: s['connected_quad_route_length']))
    else:
        remaining.append(rejection)
plan['rejected'] = remaining
plan['wrapped_parent_sha256'] = hashlib.sha256((R / 'wrapped-bark-donor-plan.json').read_bytes()).hexdigest()
plan['scope'] += ' Residual targets use generated interior donors through at most four connected quad edges, piecewise route <=1 world unit. This is an inferred bark-color continuation, not an observed source claim.'
(R / 'short-route-bark-donor-plan.json').write_text(json.dumps(plan, indent=2) + '\n')
print(json.dumps(dict(planned=len(plan['plans']), rejected=remaining, optimized=[r for r in plan['plans'] if 'connected_quad_route_length' in r])))
