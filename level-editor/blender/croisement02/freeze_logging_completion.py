"""Bind reviewed logging roots and the independent physical visibility audit."""
import json,sys
from pathlib import Path
from catalog import OUT
sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'level-editor/refinement/blender'))
from evidence_io import sha,write_json
from prop_completion_candidates import expose

def main():
    worker=OUT/'restart2-vegetation/logging-root-package-v1/assets/croisement02-logging-clearing-log';inspection=worker/'inspection'
    model=sha(worker/'model.blend')
    assert model=='4ff3e9373996f57e01cbb1f03a80398bccab89bf0d1af93410f66216c9adf91f'
    joint=OUT/'leaf-clump-joint-review/restart2-logging-root-4ff3e937';guard=joint/'physical-source-guards-per-asset-v3.json';data=json.loads(guard.read_text())
    assert data['target_counts']=={'croisement02-logging-clearing-log':2528,'croisement02-shrub-86':1}
    assert not any(n['new_root_blocks'] for n in data['neighbors'])
    assert data['minimum_world_z']>=0 and data['joint_evidence_sha256']==sha(joint/'evidence.json')
    limitations=['Inferred reverse root bark remains repetitive and awaits separate texture completion.',
        'Ground in the neighborhood review is a neutral contact guide; painted-ground integration is separate.',
        '2528 sampled former gap rays hit log/root geometry; one ray correctly hits foreground shrub86.',
        'Combined-BVH alpha traversal differed at three foliage boundary centers; independent per-asset nearest hits prove foliage lies92–149 ray units in front. Both numerical reports are preserved.',
        'The previously approved original log is unchanged. New additive root geometry requires its own user decision.']
    write_json(inspection/'root-completion-review.json',dict(status='PASS',model_sha256=model,reviewer='root coordinator',scope='Added logging roots and unchanged selected shrub86/stumps/kindling contact',
        exact_review='Root viewed logging4ff3e937 joint nine-view sheet and source-overlay now; scoped geometry/contact PASS with per-asset2528 rays + zero new native blocks, prior standalone actual8 PASS. Freeze NEXT batch card (do not expand current23), disclose inferred repetitive hidden root bark/texture still separate and one legitimate shrub86 foreground ray.',
        physical_source_guards=str(guard),physical_source_guards_sha256=sha(guard),joint_evidence_sha256=sha(joint/'evidence.json'),limitations=limitations,user_approved=False,
        secondary_evidence={str(p):sha(p) for p in [joint/'physical-source-guards-v2.json',OUT/'restart3-logging/neighbor-depth-v2/report.json']}))
    write_json(inspection/'joint-neighbourhood.json',dict(model_sha256=model,
        **{k:str(joint/v) for k,v in [('sheet','sheet.png'),('evidence','evidence.json')]},
        **{k+'_sha256':sha(joint/v) for k,v in [('sheet','sheet.png'),('evidence','evidence.json')]},
        label='New logging roots beside unchanged shrub86, logging stumps and north kindling; neutral ground guide.'))
    write_json(inspection/'visual-review.json',dict(status='PASS scoped root addition',model_sha256=model,ready_for_geometry_review=True,all_eight_actual_views_inspected=True,
        actual_materials_sha256=sha(inspection/'actual-materials/sheet.png'),root_review_sha256=sha(inspection/'root-completion-review.json'),notes=limitations,user_approved=False))
    expose(worker);print(worker)
if __name__=='__main__':main()
