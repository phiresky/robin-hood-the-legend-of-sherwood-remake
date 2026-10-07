"""Prepare private source/control bindings; pending Blender workers are never published assets."""
import json
import math
from pathlib import Path
import restart23_reconcile_state_controls as audit

ROOT, BASE, LIB = audit.ROOT, audit.OUT, audit.LIB
DEST = BASE / 'restart24-hole-mound-bindings-v1'
SOURCE = BASE / 'restart7-source-patch-delivery/contracts-v1'
PROFILES = {'Croisement01 - hole', 'Croisement01 - hiding Pc', 'Croisement02 - piege01g'}
sha, read, pin = audit.sha, audit.read, audit.pin


def main():
    assert sha(LIB / 'mission-states/index.json') == audit.CATALOG_SHA
    manifest = read(SOURCE / 'manifest.json')
    holes_path = BASE / 'restart9-hole-endpoints/receiver-audit-v2/report.json'
    holes = read(holes_path)
    mound_path = BASE / 'restart15-hiding-mounds/all-placements-v1/validation.json'
    mounds = read(mound_path)
    scatter_path = BASE / 'restart9-hiding-scatter/scatter-surfaces-v2/manifest.json'
    scatter = read(scatter_path)
    models = {
        'hole-initial': pin(BASE / 'restart9-hole-endpoints/candidate-v2/initial/model.blend'),
        'hole-applied': pin(BASE / 'restart9-hole-endpoints/candidate-v3/applied/model.blend'),
        'mounds': pin(BASE / 'restart15-hiding-mounds/all-placements-v1/model.blend'),
        'scatter': pin(BASE / 'restart9-hiding-scatter/scatter-surfaces-v2/model.blend'),
    }
    assert models['mounds']['sha256'] == mounds['model_sha256']
    assert models['scatter']['sha256'] == scatter['model_sha256']
    def pending(model, selection, placement):
        return {'kind': 'pending-worker', 'model': models[model], 'selection': selection, 'placement': placement,
                'geometry_user_approved': False, 'appearance_user_approved': False, 'runtime_asset': None}
    rows = []
    missions = {}
    for record in manifest['records']:
        if record['profile'] not in PROFILES: continue
        path = SOURCE / record['contract']
        assert sha(path) == record['sha256']
        contract = read(path)
        patch = next(p for p in contract['native']['patch_states'] if p['id'] == record['id'])
        mission_path = LIB / 'game-data/Data/Levels' / (record['mission'] + '.rhm.json')
        mission = missions.setdefault(record['mission'], {'pin': pin(mission_path), 'data': read(mission_path)})
        raw = mission['data']['mission_patches'][record['index']]
        terminal = max(1, sum(f['delay'] + 1 for f in patch['transition']) - 1)
        assert terminal == record['terminal_tick']
        aperture = None
        if record['profile'] == 'Croisement01 - hole':
            site = next((i, r) for i, r in enumerate(holes['positions']) if record['id'] in r['instances'])
            i, location = site
            assert location['display_position'] == patch['display_position']
            z = location['center']['hit'][2] if location['center']['hit'][2] > 20 else 0
            x, y = patch['display_position']
            translation = [x, -(y + math.cos(math.radians(35)) * z) / math.sin(math.radians(35)), z]
            placement = {'kind': 'blender-z-up-translation', 'value': translation, 'support_z': z, 'native_anchor_preserved': True}
            endpoints = {phase: pending('hole-' + phase, {'kind': 'all-meshes'}, placement) for phase in ['initial', 'applied']}
            aperture = {'site': f'hole-{i:02}', 'receiver': location['center']['owner'], 'trigger': record['id'],
                        'source_receiver_audit': pin(holes_path), 'initial': 'cap-closed', 'applied': 'cap-open', 'reset': 'cap-closed',
                        'current_scene_revalidation_required': True, 'unrelated_caps_unchanged': True}
        else:
            applied = next((r for r in scatter['records'] if record['id'] in r['instances']), None)
            if applied:
                endpoints = {'applied': pending('scatter', {'kind': 'exact-object-names', 'names': applied['objects']},
                    {'kind': 'saved-world-transforms', 'translation': [0, 0, 0]})}
            else:
                assert record['id'] == scatter['missing_applied_instance'] == 'mission-Tac19_FoB_EC-patch-015'
                assert not patch['integrate_in_background'] and not patch['final']
                endpoints = {'applied': {'kind': 'source-absent', 'reason': 'No final sprite and no persistent background integration'}}
            if record['profile'] == 'Croisement01 - hiding Pc':
                site = next(r for r in mounds['records'] if record['id'] in {a['id'] for a in r['aliases']})
                endpoints['initial'] = pending('mounds', {'kind': 'exact-object-names', 'names': site['objects'], 'site': site['tag']},
                                              {'kind': 'saved-world-transforms', 'translation': [0, 0, 0]})
            else:
                assert patch['initial'][0]['width'] == 4 and patch['initial'][0]['height'] == 1
                endpoints['initial'] = {'kind': 'source-absent', 'reason': 'Four-by-one native initial frame has zero alpha; pixel assertion required'}
        rows.append({'id': record['id'], 'mission': record['mission'], 'profile': record['profile'], 'source_contract': pin(path),
                     'focus_patch_id': contract['focus_patch_id'], 'source_patch_index': record['index'],
                     'terminal_tick': terminal, 'transition_duration': sum(f['delay']+1 for f in patch['transition']),
                     'initially_active': raw['active'], 'definitive': raw['definitive'], 'native_elevation': patch['elevation'],
                     'endpoints': endpoints, 'aperture': aperture,
                     'physical_transition': {'kind': 'unmodeled', 'policy': 'Use exact native artwork during transition; no mesh interpolation inferred'},
                     'scope': 'Raw pre-script source state; editor reset is explicit ForceReset, not gameplay permission',
                     'startup_condition': 'actor-dependent startup may disable this patch' if record['id'] == 'mission-Tac19_FoB_EC-patch-015' else None})
    assert len(rows) == 63 and len({r['id'] for r in rows}) == 63
    counts = {p: sum(r['profile'] == p for r in rows) for p in sorted(PROFILES)}
    assert sorted(counts.values()) == [1, 30, 32]
    result = {'schema': 'private-pending-patch-bindings.v1', 'status': 'SOURCE_PLUMBING_ONLY_GEOMETRY_AND_USER_APPROVAL_UNRESOLVED',
              'source_catalog': pin(LIB / 'mission-states/index.json'), 'current_map': pin(LIB / 'scenes/croisement02.rhlos-map.json'),
              'native_level': pin(LIB / 'game-data/Data/Levels/Croisement02.rhp.json'),
              'missions': {k:v['pin'] for k,v in missions.items()}, 'source_manifest': pin(SOURCE / 'manifest.json'),
              'authorities': [pin(holes_path), pin(mound_path), pin(scatter_path)], 'candidate_models': models,
              'counts': counts, 'bindings': rows, 'publication_allowed': False, 'production_contract': False,
              'runtime_interface': 'NativePatchPreviewContract source clocks; private endpoint intents and separate aperture cap lease. No SourceContractClockBinding, actor simulation or shared runtime changes.',
              'approval_requirements': ['Exact pending geometry decision', 'Separate required appearance decisions', 'Current receiver/contact validation', 'Exported GLB and resource hash bindings', 'Private rendered integration and root review'],
              'recipe': pin(Path(__file__).resolve())}
    DEST.mkdir(exist_ok=False)
    (DEST / 'bindings.json').write_text(json.dumps(result, indent=2)+'\n')
    print(json.dumps({'path': str((DEST/'bindings.json').relative_to(ROOT)), 'sha256': sha(DEST/'bindings.json'), 'counts': counts,
                      'bytes': (DEST/'bindings.json').stat().st_size, 'production_contract': False}))

if __name__ == '__main__': main()
