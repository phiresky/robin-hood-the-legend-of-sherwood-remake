"""Freeze new review cameras around supported shrub66; retain its earlier packet."""
import json
import sys
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT
from evidence_io import sha,write_json
from refinement_workspace import prepare,modified
from audit_candidates import audit
from render_tree import render_workspace
from render_slots import acquire,release


def main():
    asset='croisement02-shrub-66';previous=OUT/'understory-round-1/assets'/asset;worker=OUT/'understory-round-2/assets'/asset
    if any(r['asset_id']==asset and r['decision']=='approved' for r in json.loads((OUT/'user-feedback.json').read_text())['records']):raise ValueError('Approved shrub is frozen')
    if worker.exists():raise FileExistsError(worker)
    previous_hash=sha(previous/'model.blend')
    bpy.ops.wm.open_mainfile(filepath=str(previous/'model.blend'));bpy.context.preferences.filepaths.save_version=0
    prepare(worker,asset_id=asset,scene_name='Croisement02 Refinement',collection_name='Croisement02 Working',source_path=previous/'reference/source.png',
        grouping_manifest=previous/'reference/grouping.json',inventory_path=previous/'reference/inventory.json',review_path=previous/'reference/grouping-review.json',
        source_mask_manifest=previous/'source-masks.json',width=384,height=384,framing_padding=1.35)
    modified(worker);bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'))
    report=json.loads((previous/'inspection/refinement.json').read_text());report['model_sha256']=sha(worker/'model.blend');report['status']='Supported geometry retained in a fresh fitted camera packet; visual review pending'
    (worker/'inspection').mkdir(exist_ok=True);write_json(worker/'inspection/refinement.json',report)
    audit(worker);render_workspace(worker,384,release_slot=False)
    if sha(previous/'model.blend')!=previous_hash:raise ValueError('Previous worker changed during camera refit')
    write_json(worker/'inspection/refit-evidence.json',dict(previous_worker=str(previous),previous_model_sha256=previous_hash,model_sha256=sha(worker/'model.blend'),
        status='New unapproved camera packet; same supported geometry, original packet retained',reason='Source-preserving terrain support translation moved shrub outside its old oblique framing.'))


if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
