"""Prepare isolated wood display views without changing approved tree geometry."""
import json,sys
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire
from refinement_review import render_review
from refinement_workspace import _review_layers
from review_evidence import sha
R=ROOT/'level-editor/work/croisement01-refinement/restart2';w=R/'tree01-soil-joint-v10/assets/croisement01-tree-01';out=R/'tree01-isolated-wood-input-v1';assert not out.exists();before=sha(w/'model.blend');acquire();bpy.ops.wm.open_mainfile(filepath=str(w/'model.blend'));cfg=json.loads((w/'workspace.json').read_text());wood=next(o for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')==cfg['asset_id'] and o.get('source_node')=='building-029');frames=json.loads((w/'modified/views.json').read_text());result=render_review(out,scene_name=cfg['scene_name'],collection_name=cfg['collection_name'],asset_id=cfg['asset_id'],source_path=cfg['source_path'],frame_manifest=frames,width=cfg['width'],height=cfg['height'],elevation_degrees=cfg['elevation_degrees'],context_padding=cfg['context_padding'],framing_padding=cfg.get('framing_padding',1.04),lighting=cfg['lighting'],projection_layers=_review_layers(cfg),source_mask_manifest=cfg['source_mask_manifest'],render_object_names=[wood.name],allow_mask_revision=True)
result['texture_receiver_object_names']=[wood.name];(out/'views.json').write_text(json.dumps(result,indent=2)+'\n')
for old,new in zip(frames['views'],result['views']):
 for key in ['camera_matrix_world','ortho_scale','index']:assert old[key]==new[key]
assert sha(w/'model.blend')==before
(out/'derivation-proof.json').write_text(json.dumps(dict(status='Prepared input for grouped scope review; generation not authorized by this receipt',model_sha256=before,source_views_sha256=sha(w/'modified/views.json'),new_views_sha256=sha(out/'views.json'),receiver=wood.name,geometry_unchanged=True,cameras_unchanged=True,excluded='Inferred context crown omitted only from rendered display, unchanged in approved model. Soil and all neighboring objects not receivers.',reason='Prior crown-visible packet left11382off-map bough face centers unfilled. Expose exact approved hidden wood for source-preserving texture inference.'),indent=2)+'\n');print(out)
