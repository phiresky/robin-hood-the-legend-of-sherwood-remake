"""Bind final scoped southwest root review to its exact saved packet."""
import json
import sys
from pathlib import Path
from catalog import OUT
sys.path.insert(0,str(Path(__file__).resolve().parents[3]/"level-editor/refinement/blender"))
from evidence_io import sha, write_json
from prop_completion_candidates import expose


def main():
    worker=OUT/'restart2-vegetation/southwest-root-package-v1/assets/croisement02-southwest-stumps'
    inspection=worker/'inspection'
    model=sha(worker/'model.blend')
    assert model=='b650c6daa46be56eb58390a9c407a39153eca5a950044bf9e990f4554cbeea7a'
    joint=OUT/'leaf-clump-joint-review/restart2-southwest-root-b650c6da'
    guard=joint/'physical-source-guards-v2.json'
    data=json.loads(guard.read_text())
    assert data['target_counts']=={'croisement02-southwest-stumps':1502}
    assert not any(n['new_root_blocks'] for n in data['neighbors'])
    assert data['minimum_world_z']>=0
    assert data['joint_evidence_sha256']==sha(joint/'evidence.json')
    # The finite audit is preserved verbatim and independently hash-bound.
    limitations=['Ground in this joint is a neutral contact guide; full painted-ground integration remains a separate check.',
        'Repetitive yellow/brown inferred rear bark still needs texture completion.',
        'Both original approved stumps are preserved. This additive root geometry has no inherited user approval.']
    root=dict(status='PASS',model_sha256=model,reviewer='root coordinator',scope='New root geometry and unchanged selected five-neighbor joint',
        exact_review='Root SWb650c6da joint9/sourceoverlay reviewed: source neighborhood alignment coherent, roots on contactguide, no visible new neighbor conflict. Finalscopedgeometry PASS with neutralguide/provisionalground and inferredbark disclosed. Add SW geometrycard to upcomingumbrella, old approvedstumpssource unchanged.',
        joint_evidence_sha256=sha(joint/'evidence.json'),physical_source_guards=str(guard),physical_source_guards_sha256=sha(guard),limitations=limitations,user_approved=False)
    write_json(inspection/'root-completion-review.json',root)
    write_json(inspection/'joint-neighbourhood.json',dict(model_sha256=model,
        **{k:str(joint/v) for k,v in [('sheet','sheet.png'),('evidence','evidence.json')]},
        **{k+'_sha256':sha(joint/v) for k,v in [('sheet','sheet.png'),('evidence','evidence.json')]},
        label='New southwest roots with unchanged shrubs79/81, rock, fence and logpile; neutral contact guide, inferred bark pending.'))
    write_json(inspection/'visual-review.json',dict(status='PASS scoped root addition',model_sha256=model,
        ready_for_geometry_review=True,all_eight_actual_views_inspected=True,
        actual_materials_sha256=sha(inspection/'actual-materials/sheet.png'),root_review_sha256=sha(inspection/'root-completion-review.json'),
        physical_source_guards_sha256=sha(guard),limitations=limitations,user_approved=False))
    expose(worker)
    print(worker)

if __name__=='__main__': main()
