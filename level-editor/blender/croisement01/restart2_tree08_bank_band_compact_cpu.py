"""Drop the sliver collapse's unreferenced vertex, with exact triangle preservation."""
import hashlib,json
import numpy as np
from restart2_tree08_root_embedding_cpu import R
O=R/'tree08-bank-hug-affine-cpu-v4';O.mkdir(exist_ok=False)
source=R/'tree08-bank-hug-affine-cpu-v3/candidate.npz';m=np.load(source);used=np.unique(m['faces']);assert len(m['vertices'])-len(used)==1
f=np.searchsorted(used,m['faces']);v=m['vertices'][used];before=m['before_vertices'][used];assert np.array_equal(v[f],m['vertices'][m['faces']]);assert np.array_equal(before[f],m['before_vertices'][m['faces']]);np.savez_compressed(O/'candidate.npz',vertices=v,faces=f,before_vertices=before)
r=json.loads((source.parent/'report.json').read_text());r.update(candidate_sha256=hashlib.sha256((O/'candidate.npz').read_bytes()).hexdigest(),parent_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),unused_vertices_removed=1,all_physical_triangle_positions_exact=True);(O/'report.json').write_text(json.dumps(r,indent=2)+'\n');print(r['candidate_sha256'])
