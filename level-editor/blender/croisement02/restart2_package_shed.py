"""Package additive shed facade while preserving exact reviewed material bytes."""
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
    trial=OUT/'restart2-vegetation/shed-front-v6'
    previous=OUT/'scenery-round-1/assets/croisement02-woodcutters-shed'
    output=OUT/'restart2-vegetation/shed-package-v1/assets/croisement02-woodcutters-shed'
    resume=output.exists()
    if resume and not (output/'modified/views.json').exists():raise ValueError('Incomplete preparation cannot resume')
    proof=json.loads((trial/'root-review.json').read_text());model_hash=sha(trial/'model.blend')
    if proof['model_sha256']!=model_hash or not proof['status'].startswith('PASS scoped'):raise ValueError('Exact root review required')
    previous_hash=sha(previous/'model.blend')
    if previous_hash!='ebb17e8b4ed1b0e2299b53f6d4e5f645ff92a7b88337a0159e9b3dd88839cbed':raise ValueError('Approved shed changed')
    cfg=json.loads((previous/'workspace.json').read_text());report=json.loads((previous/'inspection/refinement.json').read_text())
    acquire()
    try:
        if not resume:
            bpy.ops.wm.open_mainfile(filepath=str(previous/'model.blend'));bpy.context.preferences.filepaths.save_version=0
            prepare(output,asset_id=cfg['asset_id'],scene_name=cfg['scene_name'],collection_name=cfg['collection_name'],
                    source_path=previous/'reference/source.png',grouping_manifest=previous/'reference/grouping.json',
                    inventory_path=previous/'reference/inventory.json',review_path=previous/'reference/grouping-review.json',
                    source_mask_manifest=previous/'source-masks.json',width=384,height=384,framing_padding=1.12,lighting=cfg['lighting'])
        inspection=output/'inspection';inspection.mkdir(exist_ok=True)
        shutil.copyfile(trial/'model.blend',output/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(output/'model.blend'))
        report.update(model_sha256=model_hash,status='Scoped added facade; new geometry approval pending',
            correction=dict(added_closed_boards=28,former_ground_gap_pixels_closed=2261),
            limitations=['New facade and recessed opening are pending user approval.',
                         'Existing roof, sides, posts and stump geometry/UV/materials are unchanged.',
                         'Hidden board surfaces use inferred own-native supplemental board texture.'])
        write_json(inspection/'refinement.json',report)
        modified(output)
        # Source-only projection is review evidence, not a replacement for the
        # exact independently inspected material and geometry bytes.
        shutil.copyfile(trial/'model.blend',output/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(output/'model.blend'))
        write_json(output/'validation.json',validate(output));audit(output,inferred_constant_materials=('Shed additive hidden board edges inferred dark wood',))
        for name in ['root-review.json','self-review.json','proposal.json','source-overlay.png','native-source.png']:
            shutil.copyfile(trial/name,inspection/('facade-'+name))
        shutil.copytree(trial/'actual',inspection/'facade-actual')
        record_recipe(output,__file__);record_recipe(output,Path(__file__).with_name('restart2_complete_shed_front.py'))
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
