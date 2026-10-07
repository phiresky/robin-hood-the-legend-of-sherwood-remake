"""Prepare normal-HTTP verification after the root-owned context publication."""
import json
from pathlib import Path
import restart17_initial_context_dryrun as guards

ROOT = guards.ROOT
BASE = guards.BASE
LIB = guards.LIB
SOURCE = BASE / 'restart20-initial-context-browser-v6'
OUT = BASE / 'restart22-installed-context-browser-v1'


def replace_exact(text, old, new, count=1):
    guards.require(text.count(old) == count, 'Harness replacement cardinality changed: ' + old[:100])
    return text.replace(old, new)


def adapt(run, states):
    run = replace_exact(run, str(SOURCE / 'states.mjs'), str(OUT / 'states.mjs'))
    marker = "const pathname=new URL(req.url,'http://local').pathname;"
    following = "if(req.url==='/seven-state-proof')"
    guards.require(run.count(marker) == run.count(following) == 1, 'Middleware boundary changed')
    start = run.index(marker)
    end = run.index(following, start)
    block = run[start:end]
    guards.require(block.count('Object.hasOwn(config.overlay,pathname)') == 1 and block.count('readFile(join(root,config.overlay[pathname]))') == 1, 'Unexpected interception block')
    run = run[:start] + run[end:]
    run = replace_exact(run, 'const served=new Map();', 'config.overlay={};const served=new Map();')
    run = replace_exact(run, "const overlayFile=config.overlay['/library/'+relative];const file=overlayFile?join(root,overlayFile):join(root,'level-editor/library',relative),expected=sha(await readFile(file));", "const file=join(root,'level-editor/library',relative),expected=sha(await readFile(file));")
    run = replace_exact(run, "status:'PASS_STAGED_EIGHT_INITIAL_CONTEXTS'", "status:'PASS_INSTALLED_EIGHT_INITIAL_CONTEXTS_NORMAL_HTTP'")
    run = replace_exact(run, 'Private nine-path overlay only: proposed41 catalog plus eight corrected contracts. Other resources use normal production route.', 'Actual installed41 catalog and all resources use the normal production route. No library overlay or interception.')
    run = replace_exact(run, 'PASS staged eight initial contexts', 'PASS installed eight initial contexts')
    states = replace_exact(states, "json(config.overlay['/library/'+entry.contract.path])", "json(join('level-editor/library',entry.contract.path))")
    receipt = "await writeFile(join(out,'normal-http-resources.json'),JSON.stringify(httpPins,null,2));"
    assertion = "const requiredInstalled=proposedCatalog.entries.filter(e=>config.entry_ids.includes(e.id));if(requiredInstalled.length!==8||requiredInstalled.some(e=>!httpPins.some(r=>r.path===e.contract.path&&r.sha256===e.contract.sha256)))throw Error('Missing exact installed contract HTTP receipt');"
    run = replace_exact(run, receipt, assertion + receipt)
    guards.require(run.count('config.overlay') == 1 and 'config.overlay={}' in run, 'Overlay access remains in runner')
    guards.require('config.overlay' not in states and 'overlayFile' not in run and 'Object.hasOwn(config.overlay' not in run, 'Staged interception remains')
    guards.require('PASS_STAGED_EIGHT_INITIAL_CONTEXTS' not in run, 'Staged success status remains')
    return run, states


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
    run, states = adapt((SOURCE / 'run.mjs').read_text(), (SOURCE / 'states.mjs').read_text())
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
