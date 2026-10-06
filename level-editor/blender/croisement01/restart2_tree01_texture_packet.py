"""Recover texture transport views from approved geometry without model mutation."""
import json,hashlib,sys
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire
from refinement_workspace import _render
R=ROOT/'level-editor/work/croisement01-refinement/restart2';W=R/'tree01-soil-joint-v10/assets/croisement01-tree-01';OUT=R/'approved-tree01-fill-v1/croisement01-tree-01/raw-reviewed-camera-packet-v1';sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
assert not OUT.exists();before=sha(W/'model.blend');acquire();bpy.ops.wm.open_mainfile(filepath=str(W/'model.blend'));cfg=json.loads((W/'workspace.json').read_text());_render(cfg,OUT,W/'modified/views.json');assert sha(W/'model.blend')==before
old=json.loads((W/'modified/views.json').read_text());new=json.loads((OUT/'views.json').read_text())
for a,b in zip(old['views'],new['views']):
 for key in ['camera_matrix_world','ortho_scale','index']:assert a[key]==b[key],key
(OUT/'derivation-proof.json').write_text(json.dumps(dict(status='PASS unchanged approved model and frozen cameras',model_sha256=before,approved_views_sha256=sha(W/'modified/views.json'),transport_views_sha256=sha(OUT/'views.json'),scope='Source-preserving raw packet recovery only; context crown will be protected before generation'),indent=2)+'\n')
