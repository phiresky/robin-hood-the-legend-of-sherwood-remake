"""Check the saved oak revision against its geometry recipe and grouped baseline."""
from collections import Counter
import json
from pathlib import Path
import sys

import bpy
import numpy as np

HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
from refine_northeast_oak import TARGETS,EDITOR
from stage_grouping_review import fingerprint,sha

root=Path(sys.argv[sys.argv.index('--')+1]).resolve()
stage=json.loads((root/'stage.json').read_text())
assert sha(root/'candidate.blend')==stage['worker_sha256']
baseline=EDITOR/'work/needs-review/sherwood-refinement/grouping-review/grouped-source-only.blend'
assert sha(baseline)==stage['baseline_worker_sha256']
assert sha(root/'native-volume.npz')==stage['source_geometry']['geometry_npz_sha256']
bpy.ops.wm.open_mainfile(filepath=str(baseline))
objects=list(bpy.data.collections['Sherwood Working'].objects)
original=fingerprint([o for o in objects if o.name not in TARGETS])
metadata={o.name:dict(o.items()) for o in objects}
bpy.ops.wm.open_mainfile(filepath=str(root/'candidate.blend'))
objects=list(bpy.data.collections['Sherwood Working'].objects)
assert fingerprint([o for o in objects if o.name not in TARGETS])==original
assert {o.name:o['asset_group'] for o in objects}=={n:p['asset_group'] for n,p in metadata.items()}
volume=np.load(root/'native-volume.npz');edges=Counter();count=0
for i,name in enumerate(TARGETS):
    obj=bpy.data.objects[name];faces=volume[f'faces{i}'];indices=np.unique(faces)
    coords=np.empty(len(obj.data.vertices)*3);obj.data.vertices.foreach_get('co',coords)
    assert np.allclose(np.array(obj.matrix_world),np.eye(4),atol=1e-6,rtol=0)
    assert np.allclose(coords.reshape(-1,3),volume['vertices'][indices],atol=1e-4,rtol=0)
    remap=np.full(len(volume['vertices']),-1,int);remap[indices]=np.arange(len(indices))
    assert [tuple(f.vertices) for f in obj.data.polygons]==[tuple(f) for f in remap[faces]]
    for key in ('source_node','source_obstacle','asset_group','part_name'):
        assert obj.get(key)==metadata[name].get(key),(name,key)
    for face in faces:
        for a,b in zip(face,np.roll(face,-1)):edges[tuple(sorted((int(a),int(b))))]+=1
    count+=len(coords)//3
assert all(v==2 for v in edges.values()),Counter(edges.values())
assert np.isfinite(volume['vertices']).all() and volume['vertices'][:,2].min()>=.1999
report=dict(status='PASS',worker_sha256=stage['worker_sha256'],verified_vertices=count,
    combined_surface_edges=len(edges),edges_with_other_than_two_faces=0,source_metadata_unchanged=True,
    grouping_membership_unchanged=True,all_other_geometry_uv_materials_unchanged=True)
(root/'saved-worker-verification.json').write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps(report,indent=2))
