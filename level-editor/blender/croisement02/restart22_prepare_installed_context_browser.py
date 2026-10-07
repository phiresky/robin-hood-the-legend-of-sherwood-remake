"""Prepare normal-HTTP verification after the root-owned context publication."""
import hashlib
import json
from pathlib import Path
import restart17_initial_context_dryrun as guards

ROOT = guards.ROOT
BASE = guards.BASE
LIB = guards.LIB
SOURCE = BASE / 'restart20-initial-context-browser-v6'
OUT = BASE / 'restart22-installed-context-browser-v1'


def main():
    proof = guards.read(SOURCE / 'browser/verification.json')
    guards.require(proof['status'] == 'PASS_STAGED_EIGHT_INITIAL_CONTEXTS', 'Fresh staged browser proof required')
    config = guards.read(BASE / 'restart17-initial-context-publication-v3/browser-config.json')
    expected = config['candidate_catalog']['sha256']
    guards.require(guards.sha(LIB / guards.INDEX) == expected, 'Root has not installed the exact proposed catalog')
    inputs = guards.read(SOURCE / 'inputs.json')
    pins = dict(inputs['files'])
    pins['level-editor/library/' + guards.INDEX] = expected
    for route, source in config['overlay'].items():
        destination = guards.safe(LIB, route.removeprefix('/library/'))
        guards.require(guards.sha(destination) == guards.sha(guards.safe(ROOT, source)), 'Installed overlay bytes differ: ' + route)
        pins[str(destination.relative_to(ROOT))] = guards.sha(destination)
    for path, digest in pins.items():
        guards.require(guards.sha(guards.safe(ROOT, path)) == digest, 'Post-publication dependency drift: ' + path)
    OUT.mkdir(exist_ok=False)
    (OUT / 'node_modules').symlink_to(ROOT / 'level-editor/app/node_modules', target_is_directory=True)
    run = (SOURCE / 'run.mjs').read_text().replace(str(SOURCE / 'states.mjs'), str(OUT / 'states.mjs'))
    # Remove all middleware interception; production Vite serves every library byte.
    start = run.index("const pathname=new URL(req.url,'http://local').pathname;")
    end = run.index("if(req.url==='/seven-state-proof')", start)
    run = run[:start] + run[end:]
    run = run.replace('const served=new Map();', 'config.overlay={};const served=new Map();')
    run = run.replace("status:'PASS_STAGED_EIGHT_INITIAL_CONTEXTS'", "status:'PASS_INSTALLED_EIGHT_INITIAL_CONTEXTS_NORMAL_HTTP'")
    run = run.replace('Private nine-path overlay only: proposed41 catalog plus eight corrected contracts. Other resources use normal production route.', 'Actual installed41 catalog and all resources use the normal production route. No library overlay or interception.')
    run = run.replace('PASS staged eight initial contexts', 'PASS installed eight initial contexts')
    states = (SOURCE / 'states.mjs').read_text()
    states = states.replace("json(config.overlay['/library/'+entry.contract.path])", "json(join('level-editor/library',entry.contract.path))")
    for name, contents in [('run.mjs', run), ('states.mjs', states), ('editor.tsx', (SOURCE / 'editor.tsx').read_text())]:
        (OUT / name).write_text(contents)
        pins[str((OUT / name).relative_to(ROOT))] = guards.sha(OUT / name)
    for path in [Path(__file__).resolve(), SOURCE / 'browser/verification.json']:
        pins[str(path.relative_to(ROOT))] = guards.sha(path)
    (OUT / 'runtime-baseline.json').write_bytes((SOURCE / 'runtime-baseline.json').read_bytes())
    (OUT / 'inputs.json').write_text(json.dumps({**inputs, 'files': pins}, indent=2) + '\n')
    (OUT / 'preparation.json').write_text(json.dumps({'status': 'PREPARED_NOT_LAUNCHED', 'catalog_sha256': expected,
        'staged_verification_sha256': guards.sha(SOURCE / 'browser/verification.json'), 'overlays': 0,
        'entries': 41, 'tested_corrected_entries': 8, 'pins': len(pins), 'library_modified': False}, indent=2) + '\n')
    print(json.dumps({'output': str(OUT.relative_to(ROOT)), 'pins': len(pins), 'library_modified': False}))


if __name__ == '__main__':
    main()
