"""Combine two private terrain hypotheses while retaining all unrelated receivers."""
import hashlib
import json
import shutil
import sys
from pathlib import Path
import bpy

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
from review_evidence import sha
R=ROOT/'level-editor/work/croisement01-refinement/restart2'
base=R/'tree05-regional-banks-v7/assets/croisement01-tree-05'
donor=R/'tree04-complete-banks-v4/assets/croisement01-tree-04/model.blend'
out=R/'regional-combined-banks-v1/assets/croisement01-tree-05'
assert not out.exists()
acquire(slots=2);bpy.ops.wm.open_mainfile(filepath=str(donor));bpy.context.view_layer.update()
nodes={f'building-{i:03d}' for i in range(2,6)}
reference={o.get('source_node'):[list(row) for row in o.matrix_world] for o in bpy.context.scene.objects if o.type=='MESH' and o.get('source_node') in nodes}
assert set(reference)==nodes
bpy.ops.wm.open_mainfile(filepath=str(base/'model.blend'));bpy.context.preferences.filepaths.save_version=0
cfg=json.loads((base/'workspace.json').read_text());working=bpy.data.collections[cfg['collection_name']]
def geometry(o):
    return hashlib.sha256(json.dumps([[list(o.matrix_world@v.co) for v in o.data.vertices],[list(f.vertices) for f in o.data.polygons]],separators=(',',':')).encode()).hexdigest()
targets={o.get('source_node'):o for o in working.all_objects if o.type=='MESH'}
before={node:geometry(o) for node,o in targets.items()}
with bpy.data.libraries.load(str(donor),link=False) as (src,dst):dst.objects=list(src.objects)
loaded=[o for o in dst.objects if o is not None]
for o in loaded:bpy.context.scene.collection.objects.link(o)
bpy.context.view_layer.update();changes=[]
for node in sorted(nodes):
    matches=[o for o in loaded if o.type=='MESH' and o.get('source_node')==node]
    assert len(matches)==1;source=matches[0];target=targets[node]
    assert max(abs(source.matrix_world[i][j]-reference[node][i][j]) for i in range(4) for j in range(4))<1e-5
    assert max(abs(source.matrix_world[i][j]-target.matrix_world[i][j]) for i in range(4) for j in range(4))<1e-5
    target.data=source.data.copy();assert geometry(target)==geometry(source)
    changes.append(dict(node=node,before=before[node],after=geometry(target),vertices=len(target.data.vertices),faces=len(target.data.polygons)))
for o in loaded:bpy.data.objects.remove(o,do_unlink=True)
assert all(geometry(o)==before[node] for node,o in targets.items() if node not in nodes)
(out/'modified').mkdir(parents=True);(out/'inspection').mkdir()
shutil.copy2(base/'modified/views.json',out/'modified/views.json');shutil.copy2(base/'workspace.json',out/'workspace.json')
bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'))
report=json.loads((base.parents[1]/'construction.json').read_text())
report.update(status='PRIVATE_COMBINED_HYPOTHESES; source, seams and contact require review',model_sha256=sha(out/'model.blend'),
              changed_nodes=sorted(set(report['changed_nodes'])|nodes),combine=dict(base_sha256=sha(base/'model.blend'),donor_sha256=sha(donor),changes=changes,unrelated_world_geometry_exact=True,donor_transforms_independently_reopened=True))
(out.parents[1]/'construction.json').write_text(json.dumps(report,indent=2)+'\n')
print(out)
