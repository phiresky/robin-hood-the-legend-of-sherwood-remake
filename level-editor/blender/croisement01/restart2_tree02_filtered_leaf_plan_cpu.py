"""Continue ten sampled edge texels from generated pixels on their own leaf."""
import hashlib
import json
from pathlib import Path
import numpy as np

R = Path(__file__).resolve().parents[3] / 'level-editor/work/croisement01-refinement/restart2'
P = R / 'tree02-filtered-gap-probe-v3/report.json'
report = json.loads(P.read_text())
packet = report['packets']['Tree02 inferred crown']
vertices = {int(k): np.array(v) for k, v in packet['vertices_world'].items()}
triangles = {t['face']: t for t in packet['triangles']}
leaf_for = {f: group for group in packet['groups'] for f in group}
plans, rejected = [], []
targets = {}
for witness in report['witnesses']:
    for hit in witness['hits']:
        if hit['object'] == 'Tree02 inferred crown' and hit['ownership'] == 0:
            targets.setdefault(tuple(hit['atlas_texel']), []).append(hit)
for target, hits in targets.items():
    donors = []
    for hit in hits:
        leaf = leaf_for[hit['face']]
        leaf_vertices = set(v for f in leaf for v in triangles[f]['vertices'])
        extent = np.linalg.norm(np.ptp([vertices[v] for v in leaf_vertices], axis=0))
        tri = triangles[hit['face']]
        point = np.array(hit['weights']) @ np.array([vertices[v] for v in tri['vertices']])
        for f in leaf:
            tri = triangles[f]
            transform = np.vstack([(np.array(tri['uv']) * packet['atlas_size']).T, np.ones(3)])
            for key, value in packet['texels'].items():
                if value['ownership'] != 2:
                    continue
                donor = np.array(list(map(int, key.split(','))))
                weights = np.linalg.solve(transform, np.r_[donor + .5, 1])
                if weights.min() < 1e-6:
                    continue
                donor_point = weights @ np.array([vertices[v] for v in tri['vertices']])
                distance_fraction = float(np.linalg.norm(donor_point - point) / extent)
                if distance_fraction <= .2:
                    donors.append(dict(target=list(target), donor=donor.tolist(), donor_ownership=2, faces=leaf, distance_fraction=distance_fraction))
    if donors:
        plans.append(min(donors, key=lambda x: x['distance_fraction']))
    else:
        rejected.append(list(target))
model=R/'approved-tree02-fill-v1/croisement01-tree-02/baked-v5-filtered-bark-gaps/worker.blend'
result = dict(status='CPU PLAN ONLY', model_sha256=hashlib.sha256(model.read_bytes()).hexdigest(), probe_model_sha256=report['model_sha256'], probe_sha256=hashlib.sha256(P.read_bytes()).hexdigest(), object='Tree02 inferred crown', plans=plans, rejected=rejected, scope='Only densely sampled zero-ownership texels; generated interior donor on the same two-triangle physical leaf, <=20% leaf diagonal. Preserve all alpha, source RGBA, existing fills, geometry and UVs.')
(P.parent / 'same-leaf-filter-plan.json').write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps(dict(planned=len(plans), rejected=rejected, max_distance_fraction=max(p['distance_fraction'] for p in plans))))
