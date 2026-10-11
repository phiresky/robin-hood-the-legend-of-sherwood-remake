"""Stage two approved wood assets; root may apply after private editor proof."""
import argparse
import copy
import json
from pathlib import Path
import shutil
import restart58_publish_holes_mounds as shared

io = shared.io
ROOT, LIB, BASE = shared.ROOT, shared.LIB, shared.BASE
WORK = BASE / 'approved-round-wood-integration-oct11-v1'
RUNS = WORK / 'publication'
DELTA = WORK / 'private-resource-delta.json'
PROOF = BASE / 'trees32-38-isolated-browser-oct11-v1/handoff.json'
PROOF_SHA = 'f4d57df441eb538b4dcb3974b58b40443f898c8ccb7c560f10f8107d5f097eb8'
IDS = {'croisement02-tree-32', 'croisement02-tree-38'}
sha, require, dump = io.sha, io.require, io.dump


def read(p):
    return json.loads(p.read_text())


def validate_inputs():
    require(sha(PROOF) == PROOF_SHA, 'Isolated browser proof drift')
    proof = read(PROOF)
    require(proof['status'] == 'PASS_SCOPED_PRIVATE_DELIVERY_APPEARANCE', 'Browser did not pass')
    delta = read(DELTA)
    require(sha(ROOT / delta['approval']['path']) == delta['approval']['sha256'], 'Approval drift')
    for row in delta['resources']:
        target, candidate = ROOT / row['target'], ROOT / row['candidate']
        require(row['asset_id'] in IDS and target.parent == LIB / '3d-assets/croisement02' / row['asset_id'], 'Target scope')
        require(not target.is_symlink() and target.parent.resolve() == target.parent, 'Symlink target')
        require(sha(target) == row['before_sha256'], 'Canonical resource drift: ' + str(target))
        require(sha(candidate) == row['sha256'], 'Candidate drift')
        if target.name == 'model.glb':
            require(proof['approved_models'][row['asset_id']] == row['sha256'], 'Browser model mismatch')
    return delta


def overlay(source, destination, replacements):
    destination.mkdir()
    for child in source.iterdir():
        rel = child.relative_to(LIB)
        if rel in replacements:
            (destination / child.name).symlink_to(replacements[rel])
        elif any(p.is_relative_to(rel) for p in replacements):
            overlay(child, destination / child.name, replacements)
        else:
            (destination / child.name).symlink_to(child)
    for rel, candidate in replacements.items():
        if rel.parent == source.relative_to(LIB) and not (destination / rel.name).exists():
            (destination / rel.name).symlink_to(candidate)


def prepare(folder):
    delta = validate_inputs()
    require(not folder.exists(), 'Use a new private stage directory')
    folder.mkdir()
    rows = [{'source': str(ROOT / r['candidate']), 'target': str(ROOT / r['target']),
             'old_sha256': r['before_sha256'], 'new_sha256': r['sha256']} for r in delta['resources']]
    map_target = LIB / 'scenes/croisement02.rhlos-map.json'
    map_before = sha(map_target)
    map_doc = read(map_target)
    wanted = {r['asset_id']: r for r in delta['resources'] if Path(r['target']).name == 'model.glb'}
    count = 0
    for row in map_doc['assetSources']:
        if row['id'] in wanted:
            require(row['model_sha256'] == wanted[row['id']]['before_sha256'], 'Map model pin drift')
            row['model_sha256'] = wanted[row['id']]['sha256']
            count += 1
    require(count == 2, 'Expected exactly two map sources')
    map_candidate = folder / 'croisement02.rhlos-map.json'
    dump(map_candidate, map_doc)
    rows.append({'source': str(map_candidate), 'target': str(map_target), 'old_sha256': map_before, 'new_sha256': sha(map_candidate)})
    index_target = LIB / '3d-assets/index.json'
    index_before = sha(index_target)
    text = index_target.read_text()
    decoder = json.JSONDecoder()
    replacements = []
    descriptors = []
    for aid in sorted(IDS):
        marker = '{"id":' + json.dumps(aid)
        require(text.count(marker) == 1, 'Expected unique compact catalog row')
        start = text.index(marker)
        row, end = decoder.raw_decode(text, start)
        descriptor = LIB / '3d-assets/croisement02' / aid / 'asset.json'
        require(sha(descriptor) == row['descriptor_sha256'], 'Descriptor/catalog drift')
        require(read(descriptor) == row['editor'], 'Embedded descriptor differs')
        descriptors.append(io.pin(descriptor))
        row['preview_model'] = f'croisement02/{aid}/preview.glb'
        replacements.append((start, end, json.dumps(row, separators=(',', ':'))))
    for start, end, replacement in sorted(replacements, reverse=True):
        text = text[:start] + replacement + text[end:]
    index_candidate = folder / 'index.json'
    index_candidate.write_text(text)
    del text
    rows.append({'source': str(index_candidate), 'target': str(index_target), 'old_sha256': index_before, 'new_sha256': sha(index_candidate)})
    require(sha(map_target) == map_before and sha(index_target) == index_before, 'Concurrent catalog/map drift')
    replacements = {Path(r['target']).relative_to(LIB): Path(r['source']) for r in rows}
    overlay(LIB, folder / 'library', replacements)
    plan = {'status': 'PRIVATE_EDITOR_REVIEW_PENDING', 'records': rows,
            'isolated_browser': io.pin(PROOF), 'approval': delta['approval'],
            'protected_descriptors': descriptors, 'delta': io.pin(DELTA),
            'executor': io.pin(Path(__file__).resolve()),
            'scope': 'Ten delivery resources, two map model pins, and two palette preview paths. All placements, gameplay and descriptors unchanged; unrelated catalog bytes preserved.',
            'overlay_library': str(folder / 'library'),
            'editor_checks_required': ['map_load', 'group_selection', 'part_selection', 'palette_insert', 'save_reload', 'native_contact_context']}
    dump(folder / 'plan.json', plan)
    dump(folder / 'browser-request.json', {'status': 'PRIVATE_NORMAL_EDITOR_PENDING', 'plan': io.pin(folder / 'plan.json'),
         'library': str(folder / 'library'), 'map': str(folder / 'library/scenes/croisement02.rhlos-map.json'),
         'asset_ids': sorted(IDS), 'source_parts': {'croisement02-tree-32': ['building-080','building-081','building-082'], 'croisement02-tree-38': ['building-094']},
         'isolated_request': io.pin(WORK / 'browser-request-v1/browser-request.json'), 'required_checks': plan['editor_checks_required'],
         'scope': plan['scope'], 'canonical_writes': False})
    return plan


def apply(folder, editor_path, editor_sha):
    plan = read(folder / 'plan.json')
    require(sha(editor_path) == editor_sha, 'Editor proof pin mismatch')
    editor = read(editor_path)
    require(editor.get('status', '').startswith('PASS'), 'Editor proof failed')
    require(editor.get('plan_sha256') == sha(folder / 'plan.json'), 'Editor proof must bind this exact plan')
    require(all(editor.get('editor_checks', {}).get(k) is True for k in plan['editor_checks_required']), 'Editor checks incomplete')
    rows = plan['records']
    def precommit():
        validate_inputs()
        io.core.verify_pin(plan['executor'])
        io.core.verify_pin(plan['delta'])
        require(sha(editor_path) == editor_sha, 'Editor proof drift')
        for p in plan['protected_descriptors']:
            io.core.verify_pin(p)
        for row in rows:
            require(sha(Path(row['source'])) == row['new_sha256'], 'Staged bytes changed')
        for p, digest in editor.get('evidence', {}).items():
            require(sha(ROOT / p) == digest, 'Editor evidence drift')
    precommit()
    require(shutil.disk_usage(folder).free > 2**30 + sum(Path(r['source']).stat().st_size + (Path(r['target']).stat().st_size if Path(r['target']).exists() else 0) for r in rows), 'Rollback reserve insufficient')
    def postcheck():
        io.verify(rows, 'new_sha256')
        for p in plan['protected_descriptors']:
            io.core.verify_pin(p)
    io.commit(rows, folder / 'transaction', sha(folder / 'plan.json'), precommit, postcheck)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--apply', action='store_true')
    parser.add_argument('--root-reviewed', action='store_true')
    parser.add_argument('--readers-quiescent', action='store_true')
    parser.add_argument('--editor-proof', type=Path)
    parser.add_argument('--editor-sha256')
    a = parser.parse_args()
    folder = a.output.resolve()
    require(folder.parent == RUNS, 'Wrong private stage directory')
    RUNS.mkdir(exist_ok=True)
    with io.core.locked():
        if a.apply:
            require(a.root_reviewed and a.readers_quiescent and a.editor_proof and a.editor_sha256, 'Root review, quiescent readers and editor proof required')
            apply(folder, a.editor_proof, a.editor_sha256)
        else:
            prepare(folder)
    print(json.dumps({'status': 'INSTALLED_LIVE_BROWSER_PENDING' if a.apply else 'PRIVATE_EDITOR_REVIEW_PENDING', 'output': str(folder)}))


if __name__ == '__main__':
    main()
