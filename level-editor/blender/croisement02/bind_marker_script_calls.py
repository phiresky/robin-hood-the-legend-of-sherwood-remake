"""Reconcile mission marker actor/patch calls without flattening script branches."""
import hashlib
import json
import re
from pathlib import Path
from catalog import OUT, ROOT


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def class_text(text, name):
    start = text.index('class ' + name + ' {')
    end = text.find('\nclass ', start + 1)
    return text[start:end if end != -1 else len(text)]


def main():
    root = OUT / 'net-state-source-review-v1'
    manifest = json.loads((root / 'manifest.json').read_text())
    proto_path = ROOT / 'datadirs/fullgame_gog_hackable/Data/Levels/Croisement02.rhp.json'
    proto = json.loads(proto_path.read_text())
    native_count = len(proto['animations']) + sum(bool(p['element_fx']['sprite']['frame_profile_name']) for p in proto['patches'])
    rows = []
    for row in manifest['marker_instances']:
        instance = row['instance']
        mission = instance['mission']
        mission_path = OUT / 'state-candidate-v1/missions' / (mission + '.rhm.json')
        data = json.loads(mission_path.read_text())
        assert data['element_chunk_order'][:2] == ['Patch', 'Element']
        assert data['element_group_order'] == ['Animal', 'BeamMe', 'Civilian', 'PcToRescue', 'Soldier', 'Target']
        base = native_count + sum(bool(p['element_fx']['sprite']['frame_profile_name']) for p in data['mission_patches'])
        base += len(data['civilians']) + len(data['pcs_to_rescue']) + len(data['soldiers'])
        script_path = root / 'scripts' / (mission + '.ts')
        body = class_text(script_path.read_text(), instance['target']['script_class'])
        calls = []
        for match in re.finditer(r'RecordPlayAnimFreeze\(/\*actor\*/ GetActorScript\(/\*iPosition\*/ (\d+)\), /\*iId\*/ (\d+)\)', body):
            index, action = map(int, match.groups())
            target_index = index - base
            assert 0 <= target_index < len(data['targets']), (mission, index, base)
            target = data['targets'][target_index]
            calls.append(dict(actor_index=index, target_index=target_index, action=action,
                              profile=target['profile_name'], script_class=target['script_class'],
                              line=body[:match.start()].count('\n') + 1))
        patches = []
        all_patches = proto['patches'] + data['mission_patches']
        for match in re.finditer(r'ApplyPatch\(/\*patch\*/ GetPatchScript\(/\*iPosition\*/ (\d+)\)\)', body):
            index = int(match.group(1))
            assert 0 <= index < len(all_patches)
            patch = all_patches[index]
            patches.append(dict(runtime_index=index, profile=patch['element_fx']['sprite']['profile_name'],
                                line=body[:match.start()].count('\n') + 1))
        own_index = base + instance['target_index']
        own_calls = [call for call in calls if call['actor_index'] == own_index]
        assert own_calls, (mission, instance['target']['script_class'], own_index)
        target_script = root / 'marker-classes' / (mission + '-' + instance['target']['script_class'] + '.ts')
        target_script.parent.mkdir(exist_ok=True)
        target_script.write_text(body)
        rows.append(dict(mission=mission, target_index=instance['target_index'], actor_index=own_index,
                         mission_sha256=sha(mission_path), script_sha256=sha(script_path),
                         class_source=str(target_script.relative_to(root)), class_source_sha256=sha(target_script),
                         play_animation_freeze_calls=calls, apply_patch_calls=patches,
                         branch_semantics='Calls retain class-relative source line and complete class source; lists are references, not unconditional execution order.'))
    assert len(rows) == 45
    nets = []
    for row in rows:
        rigging_calls = [call for call in row['play_animation_freeze_calls'] if 'piege' in call['profile']]
        if rigging_calls:
            assert all(call['action'] == 160 for call in rigging_calls)
            assert {patch['profile'][-1] for patch in row['apply_patch_calls']} == {'e', 'i', 'g'}
            nets.append(dict(mission=row['mission'], marker_actor_index=row['actor_index'], rigging_calls=rigging_calls,
                             patches=row['apply_patch_calls'], class_source=row['class_source']))
    assert len(nets) == 10
    report = dict(status='45 source marker bindings reconciled; actual runtime/export verification still pending',
                  proto_sha256=sha(proto_path), markers=rows, net_trigger_bindings=nets,
                  index_contract='Target actor index is preceding script-element count plus target position; mobile and player slots are not inserted before these targets.',
                  limitations=['Decompiler source is derived from the preserved JSON bytecode; reencoded SCB files are transport inputs, not original binary evidence.',
                               'Marker appearance loops and action disappearance remain source-preserved; no new gameplay implementation or permanent props are introduced.'])
    (root / 'script-call-bindings.json').write_text(json.dumps(report, indent=2) + '\n')
    print(len(rows), 'marker actor identities;', len(nets), 'net trigger families; all calls resolve')


if __name__ == '__main__':
    main()
