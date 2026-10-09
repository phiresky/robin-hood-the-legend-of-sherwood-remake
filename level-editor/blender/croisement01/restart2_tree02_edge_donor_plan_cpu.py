"""Bounded same-surface bark extrapolation plan for five rendered filter gaps."""
import collections
import hashlib
import json
import sys
from pathlib import Path
import numpy as np

ROOT = Path(__file__).resolve().parents[3]
P = ROOT / 'level-editor/work/croisement01-refinement/restart2/tree02-rendered-gap-probe-v2'
report = json.loads((P / 'report.json').read_text())
plans = []
rejected = []
for name, packet in report['packets'].items():
    assert name == 'Tree02 upper stems'
    vertices = {int(k): np.array(v) for k, v in packet['vertices_world'].items()}
    triangles = collections.defaultdict(list)
    donors = collections.defaultdict(list)
    for tri in packet['triangles']:
        points = np.array([vertices[i] for i in tri['vertices']])
        uv = np.array(tri['uv']) * packet['atlas_size']
        normal = np.cross(points[1] - points[0], points[2] - points[0])
        normal /= np.linalg.norm(normal)
        triangles[tri['face']].append((tri, points, normal))
        for key, texel in packet['texels'].items():
            if texel['ownership'] not in (1, 2):
                continue
            xy = np.array(list(map(int, key.split(','))))
            weights = np.linalg.solve(np.vstack([uv.T, np.ones(3)]), np.r_[xy + .5, 1])
            if weights.min() >= -1e-6:
                donors[tri['face']].append((xy, weights @ points, normal, texel['ownership']))
    adjacent = collections.defaultdict(set)
    for edge in packet['adjacency']:
        if len(edge['faces']) == 2:
            a, b = edge['faces']
            adjacent[a].add(b)
            adjacent[b].add(a)
    targets = collections.defaultdict(list)
    for witness in report['witnesses']:
        for hit in witness['hits']:
            if hit['ownership'] == 0 and hit['object'] == name:
                targets[tuple(hit['atlas_texel'])].append(hit)
    quad_faces = {face for face, ts in triangles.items() if len(ts) == 2 and len({v for tri, _, _ in ts for v in tri['vertices']}) == 4}
    for target, hits in targets.items():
        candidates = []
        excluded = []
        for hit in hits:
            # A quad's loop triangle index is not exported; identify the matching
            # barycentric sample by its reconstructed UV and recorded texel.
            for tri, points, normal in triangles[hit['face']]:
                weights = np.array(hit['weights'])
                tex = weights @ (np.array(tri['uv']) * packet['atlas_size'])
                if not np.array_equal(tex.astype(int), target):
                    continue
                point = weights @ points
                max_hops = 4 if '--four-edge' in sys.argv else 3 if '--three-edge' in sys.argv else 2 if '--two-edge' in sys.argv else 1
                distances = {hit['face']: 0}
                for hop in range(max_hops):
                    for first in [f for f, distance in distances.items() if distance == hop]:
                        for second in adjacent[first]:
                            distances.setdefault(second, hop + 1)
                neighborhood = set(distances)
                for face in neighborhood:
                    for xy, donor_point, donor_normal, ownership in donors[face]:
                        cosine = float(normal @ donor_normal)
                        distance = float(np.linalg.norm(donor_point - point))
                        row = dict(target=list(target), donor=xy.tolist(), donor_ownership=ownership, target_face=hit['face'], donor_face=face, shared_edge=face in adjacent[hit['face']], same_face=face == hit['face'], max_edge_hops=distances[face], surface_normal_cosine=cosine, world_distance=distance)
                        if cosine >= .8 and distance <= 1.0:
                            candidates.append(row)
                        else:
                            excluded.append(row)
        if not candidates and '--curved-quad' in sys.argv:
            # A tiny bark bend can turn more than 37 degrees between adjacent
            # quads. Continue generated bark only across that single edge;
            # do not borrow from caps/fans, source texels, or disconnected wood.
            candidates = [dict(row, inference='bounded curved-quad generated bark continuation')
                          for row in excluded if row['max_edge_hops'] <= 2
                          and row['target_face'] in quad_faces and row['donor_face'] in quad_faces
                          and row['donor_ownership'] == 2
                          and ((row['max_edge_hops'] == 1 and row['surface_normal_cosine'] > 0 and row['world_distance'] <= .6)
                               or (row['max_edge_hops'] == 2 and row['surface_normal_cosine'] >= .7 and row['world_distance'] <= .75
                                   and bool(adjacent[row['target_face']] & adjacent[row['donor_face']] & quad_faces)))]
        if candidates:
            plans.append(min(candidates, key=lambda row: (row['max_edge_hops'], row['world_distance'])))
        else:
            rejected.append(dict(target=list(target), witness_faces=sorted({h['face'] for h in hits}), closest_excluded=min(excluded, key=lambda row: row['world_distance']) if excluded else None))
output = dict(status='CPU PLAN ONLY; NO PIXELS MODIFIED', model_sha256=report['model_sha256'], probe_sha256=hashlib.sha256((P/'report.json').read_bytes()).hexdigest(), object='Tree02 upper stems', plans=plans, rejected=rejected, curved_quad_exception='When enabled: generated donor only across one shared edge between two quads, positive normal cosine, <=0.6 world units; or two edges through another quad, cosine>=0.7 and <=0.75 world units; inferred bark continuation around a sharp bend, not observed source.', scope='Only sampled zero-ownership target texels. Donors are source or generated interior texels on the same polygon or at most four shared-edge neighboring polygons when --four-edge is selected, three for --three-edge, two for --two-edge; otherwise one directly shared-edge neighboring polygon, normal cosine >=0.8 and world distance <=1. Source donors remain inferred extrapolation, never source ownership. Atlas padding cannot authorize geometry or protected-pixel edits.')
(P/('curved-quad-donor-plan.json' if '--curved-quad' in sys.argv else 'four-edge-donor-plan.json' if '--four-edge' in sys.argv else 'three-edge-donor-plan.json' if '--three-edge' in sys.argv else 'two-edge-donor-plan.json' if '--two-edge' in sys.argv else 'edge-donor-plan.json')).write_text(json.dumps(output, indent=2)+'\n')
print(json.dumps({'planned':len(plans),'rejected':len(rejected),'same_face':sum(p['same_face'] for p in plans),'max_distance':max(p['world_distance'] for p in plans)}))
