"""Reconcile scoped approvals, private exports and live physical delivery."""
import collections
import datetime
import hashlib
import json
import struct
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
OUT = REPO/'level-editor/work/croisement02-refinement'
LIB = REPO/'level-editor/library'
DEST = OUT/'restart9-completion-audit-v1'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read(path):
    return json.loads(path.read_text())


def main():
    DEST.mkdir(exist_ok=False)
    sources = {
        'catalog': OUT/'ownership-revision/catalog.json',
        'source_assignments': OUT/'restart2-vegetation/source-role-delta-v3/source-masks.json',
        'candidate': OUT/'restart2-textures/post-batch15-static-candidate-v6/candidate.json',
        'candidate_map': OUT/'restart2-textures/post-batch15-static-candidate-v6/croisement02.rhlos-map.json',
        'live_map': LIB/'scenes/croisement02.rhlos-map.json',
        'live_state_index': LIB/'mission-states/index.json',
        'live_sign_contract': LIB/'mission-states/croisement02/contracts/signposts.json',
        'state_gate': OUT/'restart2-state/post-batch15-static-compatibility-v6/verification.json',
        'seven_editor_proof': OUT/'restart2-textures/batch10-private-browser-delivery-v1/editor-review-v6/result.json',
        'seven_root_review': OUT/'restart2-textures/batch10-private-browser-delivery-v1/editor-review-v6/root-review.json',
        'seven_publication': OUT/'restart2-state/remaining-seven-publication-v1/plan.json',
        'remaining_source': OUT/'restart7-source-patch-delivery/contracts-v1/manifest.json',
        'remaining_source_runtime': OUT/'restart7-source-patch-delivery/ui-proof-v3/root-review.json',
        'physical_scope': OUT/'restart7-source-patch-delivery/physical-scope-v3/report.json',
        'ambient_sources': OUT/'animation-references/manifest.json',
        'tree41_prototype': OUT/'tree41-phase-appearance-proof-v2/proof.json',
        'tree41_status': OUT/'tree41-phase-appearance-proof-v2/animation-review-candidate.json',
        'old_tree41_superseded': OUT/'tree41-animation-proof/SUPERSEDED.json',
        'sign_export': OUT/'state-sign-candidate/animation-export-v2/verification.json',
        'sign_export_review': OUT/'state-sign-candidate/animation-export-v2/self-review.json',
        'target_inventory': OUT/'state-target-evidence/manifest.json',
        'batch16_approval': OUT/'restart3-review-batches/batch-v16/user-approval.json',
        'batch17_pending': OUT/'restart3-review-batches/batch-v17/evidence.json',
        'tree24_export': OUT/'restart8-five-bark-approved-export-v1/tree-24-v1/root-review.json',
        'residual_roles': OUT/'restart6-source-coverage/residual-role-ledger-v9.json',
        'terrain_closed': OUT/'restart4-terrain-completion-audit-v1/report.json',
        'hole_card': OUT/'restart9-hole-endpoints/paired-geometry-ready-v1/bound-members.json',
        'hole_aperture': OUT/'restart10-hole-aperture/packet-v1/runtime-proof.json',
        'appearance_limitations': OUT/'restart6-appearance-completion-audit-v1/report.json',
    }
    # Record available exact receipts; an absent optional historical receipt
    # cannot be silently promoted to proof.
    absent = [key for key,path in sources.items() if not path.is_file()]
    sources = {key:path for key,path in sources.items() if path.is_file()}
    evidence = {key:dict(path=str(path),sha256=sha(path)) for key,path in sources.items()}
    catalog, candidate, live = (read(sources[k]) for k in ['catalog','candidate_map','live_map'])
    rows = candidate['assetSources']+candidate['sceneAssets']
    models = []
    root = sources['candidate_map'].parent/'map-assets'
    for row in rows:
        path = root/row['model']
        assert sha(path) == row['model_sha256'], path
        with path.open('rb') as stream:
            header=stream.read(20)
            assert header[:4] == b'glTF'
            length,kind=struct.unpack('<II',header[12:20])
            assert kind == 0x4E4F534A
            document=json.loads(stream.read(length))
        models.append(dict(model=str(path),sha256=row['model_sha256'],
                           animation_clips=len(document.get('animations',[]))))
    owners=catalog['canonical_owners']
    missing_buildings=[f'building-{i:03}' for i in range(150) if f'building-{i:03}' not in owners]
    assert not missing_buildings, missing_buildings
    animations=read(sources['ambient_sources'])['animations']
    targets=read(sources['target_inventory'])
    profiles=read(sources['physical_scope'])['records']
    state_index=read(sources['live_state_index'])
    inventory=dict(logical_groups=len(catalog['groups']),canonical_source_nodes=len(owners),
        registered_native_building_nodes=150,missing_building_nodes=missing_buildings,
        ownership_is_not_physical_completion=True,candidate_assets=len(rows),
        candidate_placements=len(candidate['placements']),live_placements=len(live['placements']),
        live_state_entries=len(state_index['entries']),
        ambient_profiles=[dict(index=r['index'],kind=r['kind'],profile=r['profile'],frames=len(r['frames'])) for r in animations],
        target_profiles=[r['profile'] for r in targets['profiles']],
        physical_reveal_profiles=[dict(profile=r['profile'],instances=r['instances']) for r in profiles],
        candidate_glb_animations=models,
        clip_scan_limit='Zero clips in the static candidate does not inspect separate endpoint GLBs or prove absence of procedural animation. Physical ambient completion is also unsupported by current owner receipts.')
    matrix=[]
    def row(id,owner,approved,exported,live,physical,next_step,refs):
        matrix.append(dict(id=id,owner=owner,approval=approved,export=exported,live=live,
                           physical_completion=physical,next_step=next_step,evidence_keys=refs))
    row('refined-static-map','texture_packets','Scoped assets approved; pending deltas excluded',
        'Private candidate7722076d:137 assets,136 placements; final Editor proof pending',
        'Not published; current live map242a28a2 has149 legacy placements',
        'Not whole-map complete','Finish exact candidate Editor review, publish guarded transaction, verify normal delivery and final combined scene',
        ['catalog','candidate','candidate_map','live_map','state_gate'])
    row('five-approved-bark-deltas-18-24-38-39-45','approved_texture_integration -> texture_packets',
        'Batch16 appearance approved','All raw exports exist;24 derivative root PASS; other4 delivery guards/WebGL reviews ongoing',
        'Not included in candidate-v6 or live','Approved appearance, incomplete derivative delivery',
        'Finish four exact export reviews, integrate all five into subsequent static candidate', ['batch16_approval','tree24_export'])
    row('tree19-tree25-bark','remaining_geometry -> texture_packets','Geometry approved; final appearance pendingBatch17',
        'Final saved workersf715f536/0c92f9de root PASS; no final export','Not live','Appearance delivery pending',
        'After exact user decision, export/verify current final workers and integrate', ['batch17_pending'])
    row('ground49-contact-colors','terrain_domains -> texture_packets','PendingBatch17; previous ground206/14 approved',
        'Private67b6f3c2; no approved new export','Not live','Source-contact fallback only, no foreground geometry claim',
        'After decision, preserve approved terrain/known pixels and publish bounded appearance delta', ['batch17_pending','terrain_closed','residual_roles'])
    row('seven-existing-state-contracts','state_completion + texture_packets','Existing endpoints have scoped geometry/appearance approvals',
        'Private seven contracts;97 Editor checks/16 contact views; candidate-v6 semantic compatibility PASS',
        'Seven additions uninstalled; live34 controls43/129 patches, private extension covers47/129',
        'Physical endpoints exist; matching refined-scene delivery incomplete; no invented rigid motion required',
        'Publish matching static candidate then seven contracts with suppression/reset and unchanged existing-entry proof',
        ['seven_editor_proof','seven_root_review','seven_publication','state_gate','live_state_index'])
    row('remaining82-native-state-controls','state_completion','Source artwork/timing pendingBatch17',
        'Private source parity/UI proof;47+82 covers129 source controls','Uninstalled',
        'Source controls are not physical endpoint completion',
        'Resolve grouped source decision and source-only publication; retain separate physical obligations',
        ['remaining_source','remaining_source_runtime','physical_scope','batch17_pending'])
    row('holes30-instances17-positions','remaining_geometry + terrain_domains + state_completion',
        'Initial24c2c300/appliedb6528505 root PASS; grouped user geometry pending',
        'Private workers/contact proof;11 receiver triangles partitioned in aperture prototype','Not live',
        'Endpoint geometry reviewed; appearance/export/aperture controller pending',
        'Obtain geometry decision, finish unknown surfaces, export and integrate independently resetting terrain caps',
        ['hole_card','hole_aperture'])
    row('hidden-archers-five-profiles19-instances','missing_vegetation + state_completion',
        'Source evidence only; current physical receiver work private','No final physical endpoint package','Not live',
        'Foreground foliage/substrate reveal ownership and endpoints incomplete',
        'Finish source-front receiver audit, physical initial/applied foliage and substrate, contacts and state delivery', ['physical_scope'])
    row('hidingPc32-and-orphan-piege01g-scatter1','state_completion',
        'Source evidence; physical proposals in progress','No completed approved physical package','Not live',
        'Compact leaf mound/scatter and disappearing instance require physical representation',
        'Finish source-scoped leaf endpoints, receiver contacts and reset/applied behavior; do not invent a net bag for orphan scatter', ['physical_scope'])
    row('physical-signs-five-instances','UNASSIGNED physical integration; state_completion owns source/runtime',
        'Standalone source-derived sign proof exists; final independent/user scope not established by this audit',
        '970f8a4c GLB:32 poses,97 STEP channels,2.56s cycle; isolated verification PASS',
        'Live signposts contract is controlled-native-loop-preview only; no physical sign binding',
        'Model exists; five-instance physical occlusion/context delivery incomplete',
        'Reconcile exact existing approval/review, then place physical signs with ground/shadow/native ordering proof and live playback',
        ['sign_export','sign_export_review','live_sign_contract'])
    row('canopy-eight-ambient-groups','UNASSIGNED physical animation lane',
        'Static crowns approved; source phase animation approval not established',
        'Tree41 phase prototype only; neighbor ownership/runtime/tick holds explicit',
        'Native source loop exists; current refined candidate has no declared ambient animation clips',
        'Physical canopy phase behavior not established for all eight groups',
        'Audit current approved crown ownership for all eight source sequences; adapt supported phase representation, verify timing/clones/occlusion and integrate',
        ['ambient_sources','tree41_prototype','tree41_status','old_tree41_superseded','live_sign_contract'])
    row('butterfly-seven-ambient-sequences','UNASSIGNED physical ambient closure; state_completion source timing',
        'Native source frames/timing preserved','No complete physical ambient delivery receipt established',
        'Native source context is present; physical arbitrary-view status unproven',
        'Representation and full-scene playback closure needed, not automatically seven missing mesh models',
        'Audit existing ambient representation, placement and timing; establish appropriate arbitrary-view delivery without inventing source motion',
        ['ambient_sources','live_sign_contract'])
    row('45-bow-targets-and-gameplay-metadata','state_completion + texture_packets',
        'Source identities and metadata preserved','Static semantic gate covers154 obstacles/9 transitions; target context contains45 markers',
        'No complete gameplay-script execution claim','Markers are interaction controls, not missing furniture',
        'Verify final mission-scoped visibility/identity and metadata preservation; separate game-script implementation from asset refinement',
        ['target_inventory','state_gate','candidate'])
    row('accepted-source-and-inherited-quality-limitations','No new geometry writer assigned',
        'Bank24 AA; fence98ground14; fence101 correction approved; elevated25 partial39/4096 and19 single edge accepted',
        '101 and bank103 integrated in private candidate; accepted limitations remain disclosed','New candidate not live',
        'Do not inflate geometry or classify known olive/gray as missing fill;14/17 bark repetition and29/30 coarse roots retained',
        'Carry exact limitations into final review; reopen only on concrete new evidence or explicit quality assignment',
        ['residual_roles','appearance_limitations','candidate','terrain_closed'])
    report=dict(status='Read-only completion audit; map is not complete',created_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),
        evidence=evidence,missing_optional_receipts=absent,inventory=inventory,remaining_work=matrix,
        source_semantics='Integrating patches apply the last transition to the background and restore the original substrate on reset. Empty final effect rows do not mean no applied physical state.',
        exclusions=['No completion percentage: source ownership, approvals, exports, publication and animation have different denominators.',
                    'No basket requirement found in the current126-group catalog or13 target profiles; no new basket task inferred.',
                    'Legacy catalog.json68 groups and checklist-v24 contain superseded progress; current exact receipts take precedence.',
                    'This is a finite receipt/source inventory audit, not a fresh visual review of every surface or executed gameplay script.'],
        owner_reconciliation='Current statuses reconciled with texture_packets, state_completion, approved_texture_integration, terrain_domains and missing_fences. No canonical writes, renders, API calls or host inspection.')
    for key,path in sources.items():
        assert sha(path)==evidence[key]['sha256'], ('Input changed during audit',key)
    (DEST/'report.json').write_text(json.dumps(report,indent=2)+'\n')
    lines=['Croisement02 remaining-work matrix — scoped receipt audit','',
           'Current logical catalog:126 groups/207 source nodes. Private candidate:137 assets. Live map remains legacy242a28a2.',
           'Approval, export, live delivery and physical animation are separate; no percent-complete claim.','']
    for r in matrix:
        lines += [r['id']+' | '+r['owner'],'  Approval: '+r['approval'],'  Export: '+r['export'],
                  '  Live: '+r['live'],'  Remaining: '+r['next_step'],'']
    (DEST/'summary.txt').write_text('\n'.join(lines))
    print(DEST/'report.json',sha(DEST/'report.json'))


if __name__=='__main__':
    main()
