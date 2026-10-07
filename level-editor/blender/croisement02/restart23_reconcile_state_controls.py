"""Read-only live control coverage and pending physical endpoint reconciliation."""
from collections import defaultdict
from pathlib import Path
import hashlib
import json

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'level-editor/work/croisement02-refinement'
LIB = ROOT / 'level-editor/library'
DEST = OUT / 'restart23-remaining-state-controls-v1'
CATALOG_SHA = 'e9bed0e6fa08731872afd48266c93abc3edda13550ec3d444c60ef88a98b7211'


def read(p): return json.loads(p.read_text())
def sha(p):
    h = hashlib.sha256()
    with p.open('rb') as f:
        for data in iter(lambda: f.read(1024 * 1024), b''): h.update(data)
    return h.hexdigest()
def pin(p): return {'path': str(p.relative_to(ROOT)), 'sha256': sha(p)}


def main():
    catalog_path = LIB / 'mission-states/index.json'
    assert sha(catalog_path) == CATALOG_SHA
    catalog = read(catalog_path)
    assert len(catalog['entries']) == 41
    context, controlled = set(), set()
    models, contracts = {}, []
    family_controls = defaultdict(set)
    for entry in catalog['entries']:
        path = LIB / entry['contract']['path']
        assert sha(path) == entry['contract']['sha256']
        contract = read(path)
        contracts.append({'id': entry['id'], 'mission': entry['mission'], **pin(path)})
        native = contract.get('native', contract)
        rows = [*native.get('elements', []), *native.get('patch_states', []), *native.get('background_states', [])]
        by_id = {r['id']: r for r in rows}
        context.update((entry['mission'], r['source']['index']) for r in rows if r.get('source', {}).get('kind') == 'mission-patch')
        for family in contract.get('families', []):
            ids = set(sum((family.get(k, []) for k in ['element_ids', 'patch_ids', 'background_ids', 'hidden_initial_element_ids']), []))
            for identifier in ids:
                source = by_id.get(identifier, {}).get('source', {})
                if source.get('kind') == 'mission-patch':
                    key = (entry['mission'], source['index'])
                    controlled.add(key)
                    family_controls[family['id']].add(key)
            for endpoint in ['initial', 'applied']:
                endpoint_models = family.get('physical', {}).get(endpoint, [])
                if isinstance(endpoint_models, dict):
                    assert endpoint_models == {'kind': 'absent'}
                    endpoint_models = []
                for model in endpoint_models:
                    path = LIB / model['model']
                    assert sha(path) == model['model_sha256']
                    if model['model'] not in models:
                        models[model['model']] = {**pin(path), 'uses': []}
                    models[model['model']]['uses'].append({'entry': entry['id'], 'family': family['id'], 'endpoint': endpoint,
                                                        'id': model['id'], 'position': model.get('position')})
                    for resource in model.get('resources', []):
                        assert sha(LIB / resource['path']) == resource['sha256']
    source_root = OUT / 'restart7-source-patch-delivery/contracts-v1'
    source = read(source_root / 'manifest.json')
    missing = {(r['mission'], r['index']) for r in source['records']}
    assert len(context) == 129 and len(controlled) == 47 and len(missing) == 82
    assert context - controlled == missing and not (controlled & missing)
    grouped = defaultdict(list)
    for row in source['records']:
        path = source_root / row['contract']
        assert sha(path) == row['sha256']
        contract = read(path)
        patch = next(p for p in contract['native']['patch_states'] if p['id'] == contract['focus_patch_id'])
        grouped[row['profile']].append({'id': row['id'], 'mission': row['mission'], 'index': row['index'],
            'source_contract': pin(path), 'display_position': patch['display_position'], 'elevation': patch['elevation'],
            'integrates_transition': patch['integrate_in_background'], 'initial_frames': len(patch['initial']),
            'transition_frames': len(patch['transition']), 'final_frames': len(patch['final']), 'terminal_tick': row['terminal_tick']})
    hub_path = OUT / 'restart3-review-batches/pending-v17-v23-hub-v1/evidence.json'
    hub = read(hub_path)
    assert hub['approval'] == 'pending'
    cards = []
    evidence = [pin(hub_path), pin(source_root / 'manifest.json')]
    for batch in hub['batches']:
        path = OUT / 'restart3-review-batches' / batch['batch'] / 'evidence.json'
        assert sha(path) == batch['evidence_sha256']
        for card in read(path)['cards']:
            if any(word in card['card_id'] for word in ['archer', 'hiding', 'hole', 'scatter', 'remaining-native']):
                members = []
                for member in card['members']:
                    item = {k: member.get(k) for k in ['asset_id', 'scope', 'review_revision', 'model', 'model_sha256']}
                    if item['model']:
                        assert sha(Path(item['model'])) == item['model_sha256']
                    members.append(item)
                cards.append({'batch': batch['batch'], 'card_id': card['card_id'], 'status': 'pending', 'members': members})
                evidence.append(pin(path))
    physical = {
        'Croisement01 - hole': dict(priority=1, status='Geometry pending Batch18; scoped aperture browser proof exists',
            owner='No active hole writer confirmed; root must assign integration owner',
            next='After exact geometry decision: complete unknown sides/interior appearance, export pair and local terrain aperture/cap receivers, bind all30 controls and verify reset/contact in current scene.',
            candidates=['restart9-hole-endpoints/candidate-v2/initial/model.blend', 'restart9-hole-endpoints/candidate-v3/applied/model.blend'],
            receipts=['restart9-hole-endpoints/review-packet-v1/root-review.json', 'restart10-hole-aperture/browser-padding-v1/root-review.json']),
        'Croisement01 - hiding Pc': dict(priority=2, status='Initial geometry pending Batch23; terminal source-surface appearance pending Batch20',
            owner='No active hiding-mound integration writer confirmed',
            next='After scoped decisions: finish inferred mound appearance and export20site endpoints; pair31 applied scatter instances with one genuinely absent applied endpoint. Preserve actor-conditional Tac19 patch015 startup distinction.',
            candidates=['restart15-hiding-mounds/all-placements-v1/model.blend', 'restart9-hiding-scatter/scatter-surfaces-v2/model.blend'],
            receipts=['restart15-hiding-mounds/grouped-geometry-review-v1/review-packet.json', 'restart15-hiding-mounds/all-placements-v1/author-review-v1.json', 'restart9-hiding-scatter/scatter-surfaces-v2/manifest.json']),
        'Croisement02 - piege01g': dict(priority=3, status='Terminal leaf scatter source appearance pending Batch20',
            owner='Shares scatter package; no separate geometry writer needed',
            next='Export exact leaf-scatter receiver and bind initial absence/applied presence/reset; this orphan profile is not a missing net bag.',
            candidates=['restart9-hiding-scatter/scatter-surfaces-v2/model.blend'], receipts=['restart9-hiding-scatter/scatter-surfaces-v2/manifest.json']),
    }
    for number in range(1, 6):
        profile = f'Croisement02 - hidden archer{number:02}'
        if number in (3, 4):
            directory = f'restart8-hidden-archer-leaf-trial-v3/profile-{number:02}'
            status = 'Paired geometry pending Batch19; appearance/export/runtime still separate'
            next_step = 'After geometry decision: preserve owned native leaves, complete unknown reverse surfaces, export both states and bind fixed-site foliage reveal without hiding whole crowns.'
            receipts = []
        elif number == 5:
            directory = 'restart14-hidden-archer/climbing-v17/profile-05'
            status = 'Private HOLD: discontinuous stems and detached inferred leaf fans'
            next_step = 'Resolve continuous support and source-preserving attachment before new geometry review; no export or integration from held pair.'
            receipts = ['restart14-hidden-archer/climbing-v17/root-morphology-review-v1.json', 'restart14-hidden-archer/climbing-v17/exact-geometry-readonly-v2/report.json']
        else:
            directory = f'restart14-hidden-archer/candidate-v5/profile-{number:02}'
            status = 'Private HOLD: source stretched across rock/bank depth jumps'
            next_step = 'Reuse successful bounded profile05 construction only after its review; retain current source ownership and repair support before geometry review.'
            receipts = ['restart14-hidden-archer/candidate-v5/author-hold.json', 'restart14-hidden-archer/progress-v3.json']
        physical[profile] = dict(priority=4 if number in (3,4) else 5, status=status,
            owner='missing_vegetation actively owns05 only; coordinate01/02 before any new writer;03/04 integration owner not active',
            next=next_step, candidates=[directory+'-initial/model.blend', directory+'-applied/model.blend'], receipts=receipts)
    matrix = []
    for profile, rows in grouped.items():
        info = physical[profile]
        candidates = [pin(OUT / name) for name in info['candidates']]
        receipts = [pin(OUT / name) for name in info['receipts']]
        matrix.append({'profile': profile, 'instances': len(rows), 'unique_display_positions': len({tuple(r['display_position']) for r in rows}),
            **{k:v for k,v in info.items() if k not in ['candidates','receipts']}, 'candidate_models': candidates, 'evidence': receipts,
            'live_physical_binding': False, 'members': rows})
    matrix.sort(key=lambda r: (r['priority'], r['profile']))
    live_map = LIB / 'scenes/croisement02.rhlos-map.json'
    map_data = read(live_map)
    map_references = [{k:r[k] for k in ['id','model','model_sha256','descriptor','descriptor_sha256'] if k in r}
                      for r in map_data['assetSources'] + map_data['sceneAssets']]
    report = {'status': 'READ_ONLY_RECONCILIATION_NOT_WHOLE_MAP_COMPLETION', 'catalog': pin(catalog_path), 'live_map': pin(live_map),
        'counts': {'catalog_entries': 41, 'raw_initial_context_patches': 129, 'family_controlled_patches': 47,
                   'remaining_controls': 82, 'remaining_profiles': 8, 'unique_live_endpoint_glbs': len(models)},
        'denominator_note': '47 counts unique mission patches addressed by existing families, not47 distinct meshes, families or proof of continuous physical animation.129 counts raw pre-script context, not129 published controls.',
        'current_contracts': contracts, 'live_endpoint_models_verified': list(models.values()), 'current_static_model_references': map_references,
        'static_reference_validation': 'Declared current map references bound by exact map digest; this audit freshly hashes endpoint GLBs, not every unrelated static model.',
        'family_controlled_patch_counts': {k:len(v) for k,v in family_controls.items()}, 'pending_cards': cards, 'pending_evidence': evidence,
        'remaining_matrix': matrix, 'timing_and_state_constraints': [
            'Integrating patches preserve the terminal transition image as applied background and restore original substrate on reset; empty final arrays are not evidence of no physical state.',
            'HidingPc Tac19patch015 has no final sprite and genuinely disappears after its transition; initial context is pre-script and startup may disable it conditionally.',
            'All32 hidingPc states retain native elevation20 ordering; this alone never means physical height20.',
            'Source-only82 controls remain a separate pending Batch17 artifact; initial129 context restoration does not publish these controls or approve geometry.',
            'Hidden-archer foliage endpoints do not include actor model/hidden-outline delivery; that remains the existing actor lane.'],
        'owner_coordination': {'remaining_geometry':'Owns tree32/38 only, no current hole work', 'terrain_domains':'Owns butterfly07 only, no current mound/scatter/hole work', 'missing_vegetation':'Owns held profile05 only'},
        'publication_policy':'No new decisions inferred. Root remains sole library publisher; assign missing integration owners after scoped approval, reuse these candidates rather than duplicate models.'}
    assert sha(catalog_path) == CATALOG_SHA
    DEST.mkdir(exist_ok=False)
    (DEST / 'report.json').write_text(json.dumps(report, indent=2)+'\n')
    lines = ['Croisement02 state control reconciliation','Catalog '+CATALOG_SHA,'129 raw contexts;47 unique family-controlled patches;82 remaining controls.','']
    for row in matrix:
        lines += [f"{row['profile']} | {row['instances']} instances / {row['unique_display_positions']} positions | {row['status']}", '  Next: '+row['next']]
    (DEST / 'summary.txt').write_text('\n'.join(lines)+'\n')
    print(json.dumps({'report': str((DEST/'report.json').relative_to(ROOT)), 'sha256': sha(DEST/'report.json'), 'counts': report['counts'],
                      'matrix': [{'profile':r['profile'],'instances':r['instances'],'positions':r['unique_display_positions'],'status':r['status']} for r in matrix]}))

if __name__ == '__main__': main()
