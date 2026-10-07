"""Freeze an installed-scene/private-state proof without modifying either library."""
import hashlib
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
WORK = ROOT / 'level-editor/work/croisement02-refinement'
LIB = ROOT / 'level-editor/library'

def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def main():
    expected, name = sys.argv[1:]
    out = WORK / 'restart2-state' / name
    if out.exists():
        raise RuntimeError('Refusing to overwrite a frozen proof')
    document = LIB / 'scenes/croisement02.rhlos-map.json'
    assert sha(document) == expected, 'Installed map changed before preparation'
    doc = json.loads(document.read_text())
    previous = json.loads((WORK / 'restart2-textures/batch10-private-browser-delivery-v1/map-assets/scenes/croisement02.rhlos-map.json').read_text())
    resources = {}
    def pin(path):
        path = path.resolve()
        resources[str(path.relative_to(ROOT))] = sha(path)
    pin(document)
    pin(LIB / 'mission-states/index.json')
    pin(LIB / '3d-assets/index.json')
    pin(LIB / 'game-data/index.json')
    for row in doc['assetSources'] + doc['sceneAssets']:
        for field in ('model', 'descriptor'):
            path = LIB / row[field]
            assert sha(path) == row[field + '_sha256'], str(path)
            pin(path)
        for path in (LIB / row['model']).parent.iterdir():
            if path.is_file():
                pin(path)
    for folder in ('level-editor/app/src', 'level-editor/shared/src'):
        for path in (ROOT / folder).rglob('*'):
            if path.is_file() and path.suffix in ('.ts', '.tsx', '.css'):
                pin(path)
    stage = WORK / 'restart2-state/remaining-seven-package-v2'
    manifest = json.loads((stage / 'manifest.json').read_text())
    pin(stage / 'manifest.json')
    for row in manifest['files']:
        path = stage / 'library' / row['path']
        assert sha(path) == row['sha256']
        pin(path)
    for row in manifest['reused']:
        path = LIB / row['path']
        assert sha(path) == row['sha256']
        pin(path)
    pin(stage / 'library/mission-states/index.json')
    for script in ('restart6_verify_remaining_state_editor.mjs', 'restart10_verify_seven_state_editor.mjs'):
        pin(ROOT / 'level-editor/blender/croisement02' / script)
    fixture = (WORK / 'restart2-textures/batch10-private-browser-delivery-v1/editor-review.tsx').read_text()
    oldroot = '/@fs' + str(WORK / 'restart2-textures/batch10-private-browser-delivery-v1/map-assets') + '/'
    fixture = fixture.replace(oldroot, '/@fs' + str(LIB) + '/')
    fixture = fixture.replace('openHttpGameData()', "openHttpGameData(installedRoot+'game-data/')")
    old = WORK / 'restart2-state/remaining-seven-contact-v1'
    contact = json.loads((old / 'manifest.json').read_text())
    contact['static_document'] = {'url': '/@fs' + str(document), 'sha256': expected}
    for receiver in contact['receivers']:
        row = next(r for r in doc['assetSources'] + doc['sceneAssets'] if r['id'] == receiver['id'])
        if receiver['id'] != 'croisement02-terrain':
            placement = next(p for p in doc['placements'] if receiver['id'] in p['assets'])
            oldplacement = next(p for p in previous['placements'] if receiver['id'] in p['assets'])
            assert placement['transform'] == oldplacement['transform'], 'Contact transform changed'
        for field in ('model', 'descriptor'):
            receiver[field] = {'url': '/@fs' + str(LIB / row[field]), 'sha256': row[field+'_sha256']}
    out.mkdir(parents=True)
    (out / 'editor.tsx').write_text(fixture)
    (out / 'contacts').mkdir()
    for filename in ('proof.ts', 'index.html'):
        (out / 'contacts' / filename).write_bytes((old / filename).read_bytes())
    (out / 'contacts/manifest.json').write_text(json.dumps(contact, indent=2)+'\n')
    capture = (ROOT / 'level-editor/blender/croisement02/restart6_capture_state_contacts.mjs').read_text()
    capture = capture.replace("resolve('level-editor/work/croisement02-refinement/restart2-state/remaining-seven-contact-v1')", 'resolve('+json.dumps(str(out / 'contacts'))+')')
    capture = capture.replace("'./restart6_verify_remaining_state_editor.mjs'", json.dumps((ROOT / 'level-editor/blender/croisement02/restart6_verify_remaining_state_editor.mjs').as_uri()))
    (out / 'capture.mjs').write_text(capture)
    for path in out.rglob('*'):
        if path.is_file():
            pin(path)
    (out / 'inputs.json').write_text(json.dumps({'status':'PREPARED_NOT_RUN','static_map_sha256':expected,'files':resources,'scope':'Installed static scene plus seven private state entries; no library writes.'}, indent=2)+'\n')
    print(out)

if __name__ == '__main__':
    main()
