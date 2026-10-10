"""Remove a microscopic closed pocket attached along two union edges."""
import hashlib
import json
from pathlib import Path
import numpy as np
from restart2_tree08_local_fork import audit

R = Path(__file__).resolve().parents[3]/'level-editor/work/croisement01-refinement/restart2'
source = R/'tree08-inferred-shell-cleanup-v2-diagnostic/diagnostic.npz'
data = np.load(source)
v, f = data['vertices'], data['faces']
ids = [3290, 3291, 5177, 96450]
expected = [[34469,34468,34438],[34469,34438,70219],[34468,70219,34438],[34469,70219,34468]]
assert f[ids].tolist() == expected
tetra = v[f[ids]]
local = tetra-tetra.reshape(-1,3).mean(0)
volume = float(np.einsum('ij,ij->i',local[:,0],np.cross(local[:,1],local[:,2])).sum()/6)
assert 0 < volume < 1e-7
assert not any(audit(v,f[ids])[k] for k in ['nonmanifold_edges','winding_errors','zero_area'])
keep = np.ones(len(f),bool)
keep[ids] = False
used, remap = np.unique(f[keep], return_inverse=True)
vertices, faces = v[used], remap.reshape(-1,3)
check = audit(vertices,faces)
assert not any(check[k] for k in ['nonmanifold_edges','winding_errors','zero_area'])
out = R/'tree08-inferred-shell-cleanup-v3-pocket'
out.mkdir(exist_ok=False)
np.savez_compressed(out/'candidate.npz',vertices=vertices,faces=faces)
(out/'report.json').write_text(json.dumps(dict(status='TOPOLOGY_PASS_PENDING_RAYS_AND_STRICT_INTERSECTIONS',source_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),removed_tetrahedron_faces=ids,removed_tetrahedron_signed_volume=volume,removed_positions=tetra.tolist(),topology=check),indent=2)+'\n')
