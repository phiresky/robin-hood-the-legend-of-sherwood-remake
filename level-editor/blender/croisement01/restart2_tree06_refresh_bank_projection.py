"""Refresh native wood source projection against the reviewed private bank context."""
import json,hashlib,sys,shutil
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'));sys.path.insert(0,str(Path(__file__).parent))
from render_slots import acquire
from review_evidence import sha
from refinement_workspace import _reproject,_geometry
R=ROOT/'level-editor/work/croisement01-refinement/restart2';source=R/'tree06-v13/assets/croisement01-tree-06';bank=R/'tree05-regional-banks-v15/assets/croisement01-tree-05/model.blend';out=R/'tree06-native-bank-projection-v2/assets/croisement01-tree-06';assert not out.exists();acquire(slots=2);out.mkdir(parents=True);(out/'inspection').mkdir();(out/'modified').mkdir();shutil.copy2(source/'modified/views.json',out/'modified/views.json');cfg=json.loads((source/'workspace.json').read_text());(out/'workspace.json').write_text(json.dumps(cfg,indent=2)+'\n');bpy.ops.wm.open_mainfile(filepath=str(source/'model.blend'));bpy.context.preferences.filepaths.save_version=0
collection=bpy.data.collections[cfg['collection_name']];protected={o.name:_geometry(o) for o in collection.all_objects if o.type=='MESH' and o.get('asset_group')==cfg['asset_id']};before=sha(source/'model.blend');nodes=['building-000',*[f'building-{i:03d}' for i in range(2,8)]]
with bpy.data.libraries.load(str(bank),link=False) as (a,b):b.objects=a.objects
replaced=[]
for linked in b.objects:
 if linked is not None:bpy.context.scene.collection.objects.link(linked)
bpy.context.view_layer.update()
for obj in b.objects:
 if obj is None:continue
 if obj.type=='MESH' and obj.get('source_node') in nodes:
  matches=[o for o in collection.all_objects if o.get('source_node')==obj.get('source_node')];assert len(matches)==1;target=matches[0];assert target.matrix_world==obj.matrix_world;target.data=obj.data.copy();replaced.append(obj['source_node'])

for obj in b.objects:
 if obj is not None:bpy.data.objects.remove(obj,do_unlink=True)
assert sorted(replaced)==sorted(nodes);assert protected=={o.name:_geometry(o) for o in collection.all_objects if o.name in protected};_reproject(cfg,out/'projection-context-refresh');assert protected=={o.name:_geometry(o) for o in collection.all_objects if o.name in protected};bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'));assert sha(source/'model.blend')==before
(out.parent.parent/'refresh.json').write_text(json.dumps(dict(status='SOURCE_PROJECTION_REFRESH_REQUIRES_SAVED_VIEW_REVIEW',wood_geometry_exact=True,source_model_sha256=before,model_sha256=sha(out/'model.blend'),bank_sha256=sha(bank),context_nodes=replaced,limitations='Private inferred bank context; not approval or final texture. Original source mask inventory and source image unchanged.'),indent=2)+'\n')
import render_candidate;sys.argv=['render','--',str(out)];render_candidate.main()
