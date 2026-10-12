"""Try bounded vertex identification after exact branch union; retain failures."""
import hashlib
import json
from pathlib import Path
import numpy as np
from scipy.spatial import cKDTree
from restart2_tree08_local_fork import audit

R = Path(__file__).resolve().parents[3]/'level-editor/work/active/croisement01/restart2'
source = R/'tree08-connected-tip-union-v2-unfaired/candidate.npz'
data = np.load(source)
v, f = data['vertices'], data['faces']
out = R/'tree08-connected-tip-cleanup-v2'
out.mkdir(exist_ok=False)
trials = []
for tolerance in [1e-7, 1e-5, 2.5e-5, .0001]:
    pairs = sorted(cKDTree(v).query_pairs(tolerance))
    parent = np.arange(len(v))
    def root(i):
        while parent[i] != i:
            parent[i] = parent[parent[i]]
            i = parent[i]
        return i
    for a,b in pairs:
        a,b = root(a),root(b)
        parent[max(a,b)] = min(a,b)
    mapping = np.array([root(i) for i in range(len(v))])
    faces = mapping[f]
    repeated = (faces[:,0]==faces[:,1]) | (faces[:,1]==faces[:,2]) | (faces[:,0]==faces[:,2])
    faces = faces[~repeated]
    used, remap = np.unique(faces, return_inverse=True)
    vertices, faces = v[used], remap.reshape(-1,3)
    check = audit(vertices, faces)
    movement = float(np.linalg.norm(v-v[mapping], axis=1).max())
    trial = dict(tolerance=tolerance, merged_vertices=len(v)-len(used), removed_faces=int(repeated.sum()), maximum_vertex_displacement=movement, topology=check)
    trials.append(trial)
    print(json.dumps(trial), flush=True)
    if not any(check[k] for k in ['nonmanifold_edges','winding_errors','zero_area']) and movement <= .0002:
        np.savez_compressed(out/'candidate.npz', vertices=vertices, faces=faces)
        break
np.savez_compressed(out/'last-trial.npz', vertices=vertices, faces=faces)
report = dict(status='TOPOLOGY_PASS_SOURCE_RECHECK_REQUIRED' if (out/'candidate.npz').exists() else 'HOLD_NUMERICAL_TOPOLOGY', source_sha256=hashlib.sha256(source.read_bytes()).hexdigest(), trials=trials)
(out/'report.json').write_text(json.dumps(report, indent=2)+'\n')
