"""Freeze a fresh bank geometry review without replacing prior approved history."""
import json
from pathlib import Path
import shutil
import sys
import bpy
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json,record_recipe
from render_slots import acquire,release
from refinement_workspace import prepare,modified,validate
from audit_candidates import audit
from render_tree import render_workspace
from render_tree_prototype_comparison import main as compare


def main():
    trial=OUT/'restart2-bank321/foot-candidate-v2'
    previous=OUT/'terrain-bank-candidate/assets/croisement02-north-woodland-bank'
    output=OUT/'restart2-bank321/packaged-v1/assets/croisement02-north-woodland-bank'
    if output.exists():raise FileExistsError(output)
    proof=json.loads((trial/'root-review.json').read_text());model_hash=sha(trial/'worker.blend')
    if proof['model_sha256']!=model_hash or proof['status']!='ready-for-user-new-geometry-review':raise ValueError('Exact root review required')
    previous_hash=sha(previous/'model.blend')
    if previous_hash!='5fefeb90ef39a274b9f1493c570cd98b0fc9c38d073118458c677600036f66d0':raise ValueError('Approved bank changed')
    cfg=json.loads((previous/'workspace.json').read_text());report=json.loads((previous/'inspection/refinement.json').read_text())
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(previous/'model.blend'));bpy.context.preferences.filepaths.save_version=0
        prepare(output,asset_id=cfg['asset_id'],scene_name=cfg['scene_name'],collection_name=cfg['collection_name'],
                source_path=previous/'reference/source.png',grouping_manifest=previous/'reference/grouping.json',
                inventory_path=previous/'reference/inventory.json',review_path=previous/'reference/grouping-review.json',
                source_mask_manifest=previous/'source-masks.json',width=384,height=384,framing_padding=1.12,lighting=cfg['lighting'])
        inspection=output/'inspection';inspection.mkdir(exist_ok=True)
        shutil.copyfile(trial/'worker.blend',output/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(output/'model.blend'))
        report.update(model_sha256=model_hash,status='Scoped foot correction; new geometry approval pending',
            correction=dict(moved_vertices=11,part='building-000',true_gap_pixels_closed=32,maximum_source_displacement=5.596),
            limitations=['Old bank geometry approval does not apply to this new revision.',
                         'Existing generated material is retained for actual inspection; prior texture approval is not inherited.',
                         '217 historical narrow edge misses remain; neutral material/source-role issues in whole-scene audit are separate.'])
        write_json(inspection/'refinement.json',report)
        modified(output)
        # Source-only projection is review evidence, not a replacement for the
        # exact independently inspected material and geometry bytes.
        shutil.copyfile(trial/'worker.blend',output/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(output/'model.blend'))
        write_json(output/'validation.json',validate(output));audit(output)
        for name in ['validation.json','root-review.json','visual-review.json','native-compare-0.png','native-compare-1.png','remaining-missing.png']:
            shutil.copyfile(trial/name,inspection/('foot-'+name))
        shutil.copytree(trial/'contact-review',inspection/'foot-contact-review')
        record_recipe(output,__file__);record_recipe(output,Path(__file__).with_name('correct_bank_foot.py'))
        write_json(inspection/'prototype-preservation.json',dict(previous_worker=str(previous),previous_model_sha256=previous_hash,
            previous_model_unchanged=True,model_sha256=model_hash,trial_worker=str(trial),trial_model_sha256=model_hash,
            root_review_sha256=sha(trial/'root-review.json'),approval='pending',prior_texture_approval_inherited=False))
        if sha(output/'model.blend')!=model_hash or sha(previous/'model.blend')!=previous_hash:raise ValueError('Frozen model changed')
    finally:release()
    render_workspace(output,384)
    compare(output)
    if sha(output/'model.blend')!=model_hash:raise ValueError('Packaging changed candidate')
    print(output)


if __name__=='__main__':main()
