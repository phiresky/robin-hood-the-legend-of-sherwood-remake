"""Record bounded mission playback and physical source-camera evidence."""
import json, sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'refinement/blender'))
from catalog import OUT
from evidence_io import sha, write_json


def evidence(name):
    return dict(path=name, sha256=sha(OUT / name))


def main():
    base = OUT / 'restart2-state'
    proof = base / 'sign-scene-projection-v1'
    report = json.loads((proof / 'report.json').read_text())
    imports = json.loads((proof / 'evaluated-imports.json').read_text())['imports']
    write_json(proof / 'self-review.json', dict(
        status='Physical context reviewed; native occlusion parity HOLD',
        report_sha256=sha(proof / 'report.json'),
        inspected=[f'target-{i}/comparison.png' for i in range(4, 9)],
        geometry_imports=dict(count=len(imports), local_mesh_uv_provenance_exact=all(r['local_mesh_uv_provenance_exact'] for r in imports), maximum_world_vertex_error=max(r['maximum_world_vertex_error'] for r in imports), maximum_evaluated_matrix_error=max(r['maximum_source_matrix_error'] for r in imports)),
        findings=[
            'Targets4 and6 retain all physical body samples in four tested phases.',
            'Target5 loses26–53 body samples to foreground geometry; native body is unobscured.',
            'Canonical target7 loses77–204 body samples. The private shrub57 correction is deliberately not selected here.',
            'Target8 loses490–671 body samples: most of the sign is physically hidden, exceeding the visible native frame0 foliage overlay. Canopy geometry approval does not resolve ordering.',
            'Painted-shadow parity is independent of the body object-index masks and is not passed by this proof.'
        ],
        limits=report['limitations'] + [
            'Four sign poses only, each against overlay frame0. The separate native ordering reference exhausts relative phases; this physical proof does not.',
            'No per-pixel excess-occlusion count is claimed: physical body silhouette and native sprite silhouette differ.',
            'All frozen receivers intersecting these native-camera crops are included; this is a projection subset, not a saved replacement full scene.'
        ]))
    old = base / 'editor-state-integration-plan-v5.json'
    plan = json.loads(old.read_text())
    plan['prior_plan_sha256'] = sha(old)
    plan['status'] = 'Hash-pinned mission target layer and viewport playback verified; canonical contracts, source-order presentation and state receivers remain open'
    plan['steps'][2].update(status='PASS catalog preview and explicit viewport target playback capabilities; canonical activation wiring remains separate', evidence='restart2-state/mission-player-browser-v1/verification.json', commit='e027e12e8')
    plan['steps'][3].update(status='PASS bounded target contract/layer capability', evidence='restart2-state/mission-player-browser-v1/verification.json', commit='e027e12e8', deliverable='Exact mission/level/target hashes, source physical height and separate action/sort anchor; successful refined loads alone suppress original targets. Failure, pending mission switches and disposal preserve or restore native fallback.')
    plan['coordination'] = 'Assigned mission-state modules, mission.ts and editor-viewport.ts committed e027e12e8. Existing mission serialization and UI activation unchanged.'
    plan['isolated_player_module']['status'] = 'Production loaders, catalog preview and explicit mission target viewport playback verified; full physical scene semantics remain separate.'
    plan['existing_capabilities'][-1] = 'Explicit refined mission target layer now supports hash-pinned source identities, action selection and deterministic playback.'
    plan['loader_api']['limits'][-1] = 'Viewport owns its existing single clock; callers must provide an exact validated mission-state contract. No automatic canonical contract catalog or script simulation.'
    plan['sign_context_supersession']['scope'] = 'Old unevaluated bank contexts remain superseded. Current frozen-scene projection subset verifies39 evaluated imports and reproduces canonical5/7/8 ordering conflicts. Private sibling shrub57v9 has separate scoped body evidence and unresolved painted shadow.'
    plan['sign_physical_evidence'] = evidence('restart2-state/sign-scene-projection-v1/self-review.json')
    plan['sign_presentation_contract']['runtime_status'] = 'Physical target loading/playback implemented; native-order compositor remains contract-only.'
    write_json(base / 'editor-state-integration-plan-v6.json', plan)
    old = base / 'state-completion-checklist-v12/manifest.json'
    checklist = json.loads(old.read_text())
    checklist['prior_checklist_sha256'] = sha(old)
    for row in checklist['items']:
        if row['id'] == 'log-trap':
            row['private_candidate_review'] = 'Corrected21-log six-course initial pile and unchanged fallen endpoint now explicitly user geometry approved; old inclined-layer initial remains rejected.'
            row['verified_milestones'].append('Corrected9ed5b9c2 pile source/native-first/end-on support views independently reviewed and user approved. Both guarded endpoint fill input packets prepared and delegated to texture lane.')
            row['remaining'][0] = 'Complete guarded appearance fills and saved-model review of approved corrected initial and unchanged fallen endpoints. Native silhouette219 missing/456 excess remains disclosed; no temporal identity invented.'
            row['evidence'].extend([evidence('restart2-state/user-log-pile-geometry-approval-v1.json'), evidence('restart2-state/private-log-pile-inputs-v1/manifest.json')])
        if row['id'] == 'mission-signposts':
            row['verified_milestones'].append('Actual five exported sign clones verified at exact native physical anchors with independent phase selection, loop, mission switch, failure fallback and viewport cleanup; neutral browser scene only.')
            row['remaining'][0] = 'Resolve or explicitly present native-versus-physical occlusion for targets5/7/8. Evaluated frozen-scene subset proof is complete; canonical57 remains selected. Private57v9 body fix and painted-shadow limitations remain separate.'
            row['remaining'][1] = 'Bind final approved sign/context exports into canonical mission contract catalog and editor activation. Existing validated layer alone does not finish publication, source-order compositor or scripts.'
            row['evidence'].extend([evidence('restart2-state/mission-player-browser-v1/verification.json'), evidence('restart2-state/sign-scene-projection-v1/self-review.json')])
        assert row['status'] == 'incomplete'
    checklist['integration_plan'] = evidence('restart2-state/editor-state-integration-plan-v6.json')
    checklist['mission_player'] = dict(commit='e027e12e8', evidence=evidence('restart2-state/mission-player-browser-v1/verification.json'), scope='Explicit hash-pinned mission target layer; actual five clones in neutral scene. Not canonical publication or full-scene occlusion parity.')
    checklist['geometry_review_request'] = dict(status='User approved corrected log pile and paired unchanged fallen endpoint, rock endpoints, occupied net and cleared fence. Old rejected log initial remains rejected.', evidence=[evidence('restart2-state/user-state-geometry-decisions-v2.json'), evidence('restart2-state/user-log-pile-geometry-approval-v1.json')])
    checklist['isolated_player']['status'] = 'Production loader, catalog preview and explicit mission target viewport capabilities verified; receiver transitions, native ordering, canonical contracts and physical temporal correspondence remain open.'
    checklist['status'] = 'Finite state checklist v13; corrected log approval, mission player and actual physical sign context recorded. All11 integrated workstreams remain incomplete.'
    dest = base / 'state-completion-checklist-v13'
    dest.mkdir(exist_ok=False)
    write_json(dest / 'manifest.json', checklist)


if __name__ == '__main__':
    main()
