"""Reconcile current C1 resource closure without equating it with visual completion."""
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
LIB = ROOT / 'level-editor/library'
WORK = ROOT / 'level-editor/work/active/croisement01'
AUDIT = ROOT / 'level-editor/work/active/croisement02/four-map-completion-audit-oct11-v1'


def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def read(path):
    return json.loads(path.read_text())


def main():
    out = WORK / 'restart2/completion-closure-oct11-v1'
    out.mkdir(exist_ok=False)
    map_path = LIB / 'scenes/croisement01.rhlos-map.json'
    before = sha(map_path)
    document = read(map_path)
    source = read(AUDIT / 'croisement01-source-checklist.json')
    prior = read(AUDIT / 'croisement01-asset-checklist.json')
    previous = {a['asset_id']: a for a in prior['assets']}
    assets = []
    for entry in document['assetSources'] + document['sceneAssets']:
        descriptor_path = LIB / entry['descriptor']
        descriptor = read(descriptor_path)
        pins = []
        for kind in ('model', 'descriptor'):
            path = LIB / entry[kind]
            actual = sha(path)
            pins.append(dict(path=entry[kind], expected=entry[kind + '_sha256'],
                             actual=actual, match=actual == entry[kind + '_sha256']))
        for resource in descriptor.get('resources', []):
            actual = sha(LIB / resource['path'])
            pins.append(dict(path=resource['path'], expected=resource['sha256'],
                             actual=actual, match=actual == resource['sha256']))
        gameplay = descriptor.get('gameplay', {})
        assets.append(dict(id=entry['id'], pins=pins,
                           source_nodes=[p['node'] for p in descriptor.get('parts', [])],
                           gameplay_counts={k: len(v) for k, v in gameplay.items() if isinstance(v, list)},
                           gameplay_draft=gameplay.get('draft'),
                           prior_scoped_receipts=previous.get(entry['id'], {}).get('approval_receipt_references', []),
                           completion='RESOURCE_BINDING_ONLY; source ownership, scoped approval and current runtime proof require separate reconciliation'))
    catalog_path = LIB / 'mission-states/index.json'
    entries = [e for e in read(catalog_path)['entries'] if e['map'].lower() == 'croisement01']
    state_audit = read(WORK / 'state-audit-v1/manifest.json')
    result = dict(status='INCOMPLETE_MAP; concrete remaining work identified',
                  map_sha256=before, map_stable=sha(map_path) == before,
                  counts=source['counts'], assets=assets,
                  all_declared_resource_pins_match=all(p['match'] for a in assets for p in a['pins']),
                  state_catalog_sha256=sha(catalog_path), current_state_entries=entries,
                  source_mission_count=state_audit['mission_count'],
                  source_patch_controls=state_audit['mission_patch_count'],
                  decoded_source_frames=state_audit['decoded_frame_count'],
                  source_profile_counts=state_audit['profile_counts'],
                  native_animation_profiles=[r['source_record']['sprite']['profile_name'] for r in source['animation_checklist']],
                  actions=[
                      dict(scope='Tree06 and regional banks', state='ACTIVE_PRIVATE_GEOMETRY',
                           evidence='regional-recovery-checkpoint-v2.json',
                           task='Restore 101 uncovered bark samples including 38 interior samples; correct 12 bank occlusions and rerun regional contact/source guards.'),
                      dict(scope='Tree08 wood', state='FINAL_TEXTURE_READY_PENDING_USER',
                           task='Publish exact approved wood after final texture decision; canopy remains separate.'),
                      dict(scope='Tree02', state='ROOT_PUBLICATION_PENDING',
                           task='Root applies guarded compact catalog transaction then sole browser owner proves installed scene.'),
                      dict(scope='Terrain and deferred mixed foliage', state='INCOMPLETE',
                           task='Resolve every held regional bank, ground receiver, deferred source domain and retained generic group; scoped wood approval does not cover foliage.'),
                      dict(scope='Native animations', state='CURRENT_CONTROL_COVERAGE_UNPROVEN',
                           task='Bind all 13 source profiles, alpha, delay+1 frame clocks and display-order semantics to native editor presentation and physical receivers.'),
                      dict(scope='Mission controls', state='NO_C1_ENTRIES_IN_CURRENT_STATE_CATALOG' if not entries else 'RECONCILE_ENTRIES',
                           task='Prepare scoped controls for all 108 mission patches across six missions, including background terminal frame retention, state geometry and physical/runtime proof.'),
                      dict(scope='Native nonvisual patches', state='MOVEMENT_METADATA_ONLY_UNTIL_RUNTIME_PROOF',
                           task='Verify all six native patches, mask/sight activation, movement transitions and definitive application in installed runtime.')],
                  limitations=['Catalog entries count preview routes, not gameplay completion.',
                               'A retained generic group is not assumed approved by matching source IDs.',
                               'Prior receipt list is evidence discovery only, not scope or geometry equivalence.',
                               'This CPU audit does not replace saved-model visual review or browser execution.'])
    if not result['map_stable'] or not result['all_declared_resource_pins_match']:
        raise RuntimeError('Current map or resource pins changed during closure audit')
    (out / 'report.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(dict(assets=len(assets), current_controls=len(entries),
                          resource_pins=sum(len(a['pins']) for a in assets),
                          report_sha256=sha(out / 'report.json'))))


if __name__ == '__main__':
    main()
