"""Standard root-completion packet; exact native material worker stays frozen."""
import argparse
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


def main(kind):
    logging=kind=='logging';asset='croisement02-logging-clearing-log' if logging else 'croisement02-southwest-stumps'
    trial=OUT/'restart2-vegetation'/('logging-convex-v7' if logging else 'southwest-convex-v3')
    previous=OUT/'scenery-round-1/assets'/asset
    output=OUT/'restart2-vegetation'/(kind+'-root-package-v1')/'assets'/asset
    if output.exists():raise FileExistsError(output)
    proof=json.loads((trial/'root-review.json').read_text());model_hash=sha(trial/'model.blend')
    if proof['model_sha256']!=model_hash or not proof['status'].startswith('PASS scoped'):raise ValueError('Exact root standalone review required')
    previous_hash=sha(previous/'model.blend');cfg=json.loads((previous/'workspace.json').read_text());report=json.loads((previous/'inspection/refinement.json').read_text())
    bpy.ops.wm.open_mainfile(filepath=str(previous/'model.blend'));bpy.context.preferences.filepaths.save_version=0
    prepare(output,asset_id=cfg['asset_id'],scene_name=cfg['scene_name'],collection_name=cfg['collection_name'],
        source_path=previous/'reference/source.png',grouping_manifest=previous/'reference/grouping.json',
        inventory_path=previous/'reference/inventory.json',review_path=previous/'reference/grouping-review.json',
        source_mask_manifest=previous/'source-masks.json',width=384,height=384,framing_padding=1.25,lighting=cfg['lighting'])
    inspection=output/'inspection';inspection.mkdir(exist_ok=True)
    shutil.copyfile(trial/'model.blend',output/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(output/'model.blend'))
    report.update(model_sha256=model_hash,status='Added root branches; new geometry approval pending',
        limitations=['Closed inferred branch depths fitted to native source artwork; original prop preserved.',
                      'Inferred rear bark remains repetitive and needs separate texture review.',
                      'Current ground/neighbor joint remains required for geometry readiness.'])
    write_json(inspection/'refinement.json',report);modified(output)
    shutil.copyfile(trial/'model.blend',output/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(output/'model.blend'))
    write_json(output/'validation.json',validate(output));audit(output)
    for name in ['root-review.json','self-review.json','evidence.json']:
        shutil.copyfile(trial/name,inspection/('root-completion-'+name))
    # Already independently inspected full bounds are retained, with no new
    # standalone material render or alteration of the saved reviewed worker.
    shutil.copytree(trial/'full-review',inspection/'root-completion-full-review')
    record_recipe(output,__file__)
    write_json(inspection/'prototype-preservation.json',dict(previous_worker=str(previous),previous_model_sha256=previous_hash,
        previous_model_unchanged=True,model_sha256=model_hash,trial_worker=str(trial),trial_model_sha256=model_hash,
        root_review_sha256=sha(trial/'root-review.json'),approval='pending',prior_texture_approval_inherited=False))
    if sha(output/'model.blend')!=model_hash or sha(previous/'model.blend')!=previous_hash:raise ValueError('Frozen model changed')
    print(output)


if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('kind',choices=['logging','southwest']);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    acquire()
    try:main(args.kind)
    finally:release()
