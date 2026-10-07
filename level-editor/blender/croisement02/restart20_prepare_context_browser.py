"""Prepare an isolated eight-context browser proof. Does not launch or publish."""
import hashlib
import json
from pathlib import Path

ROOT = Path.cwd()
STATE = ROOT / 'level-editor/work/croisement02-refinement/restart2-state'
OLD = STATE / 'installed41-normal-http-chunk-1-v1'
PLAN = STATE / 'restart17-initial-context-publication-v3'
OUT = STATE / 'restart20-initial-context-browser-v6'
sha = lambda data: hashlib.sha256(data).hexdigest()

def main():
    config = json.loads((PLAN / 'browser-config.json').read_text())
    plan = json.loads((PLAN / 'plan.json').read_text())
    assert sha((PLAN / 'plan.json').read_bytes()) == config['plan']['sha256']
    assert len(config['overlay']) == 9 and len(config['entry_ids']) == 8
    proposed = json.loads((PLAN / 'proposed41-index.json').read_text())
    baseline = json.loads((PLAN / 'baseline41-index.json').read_text())
    changes = [e['id'] for e, b in zip(proposed['entries'], baseline['entries']) if e != b]
    assert changes == config['entry_ids'] or set(changes) == set(config['entry_ids'])
    pins = {str(Path('level-editor/library') / p): h for p, h in plan['current_library_pins'].items()}
    catalog_audit_path = STATE / 'restart20-initial-context-browser-v5/catalog-delta-audit.json'
    catalog_audit = json.loads(catalog_audit_path.read_text())
    catalog_key = 'level-editor/library/3d-assets/index.json'
    assert catalog_audit['expected_baseline_match'] and catalog_audit['croisement02_unchanged']
    assert not catalog_audit['added'] and not catalog_audit['removed']
    assert {r['id'] for r in catalog_audit['changed']} == {'york-east-riverside-curtain-wall', 'york-market-southwest-connecting-stairs'}
    assert pins[catalog_key] == catalog_audit['baseline_sha256']
    assert sha((ROOT / catalog_key).read_bytes()) == catalog_audit['current_sha256']
    pins[catalog_key] = catalog_audit['current_sha256']
    pins[str(catalog_audit_path.relative_to(ROOT))] = sha(catalog_audit_path.read_bytes())
    drift = []
    runtime_paths = set(plan['current_runtime_pins'])
    for directory in ['level-editor/app/src', 'level-editor/shared/src']:
        runtime_paths.update(str(p.relative_to(ROOT)) for p in (ROOT / directory).rglob('*') if p.is_file() and p.suffix in {'.ts', '.tsx', '.js', '.mjs', '.json', '.wgsl', '.glsl', '.css'})
    for p in sorted(runtime_paths):
        h = plan['current_runtime_pins'].get(p)
        actual = sha((ROOT / p).read_bytes())
        if actual != h:
            drift.append({'path': p, 'expected': h, 'actual': actual})
        pins[p] = actual
    for p, h in list(pins.items()):
        assert sha((ROOT / p).read_bytes()) == h, p
    for p in config['overlay'].values():
        pins[p] = sha((ROOT / p).read_bytes())
    for p in [PLAN / 'plan.json', PLAN / 'browser-config.json', PLAN / 'baseline41-index.json', Path(__file__).resolve()]:
        pins[str(p.relative_to(ROOT))] = sha(p.read_bytes())
    OUT.mkdir(exist_ok=False)
    (OUT / 'node_modules').symlink_to(ROOT / 'level-editor/app/node_modules', target_is_directory=True)
    editor = (OLD / 'editor.tsx').read_text()
    (OUT / 'editor.tsx').write_text(editor)
    oracle_path = STATE / 'restart16-initial-context-v1/verification.json'
    oracle = json.loads(oracle_path.read_text())
    pins[str(oracle_path.relative_to(ROOT))] = sha(oracle_path.read_bytes())
    states = (OLD / 'states.mjs').read_text()
    states = states.replace("const stage=join(base,'remaining-seven-package-v2');", 'const config='+json.dumps(config)+';const oracle='+json.dumps(oracle)+';')
    states = states.replace("const manifest=await json(join(stage,'manifest.json')),checks=[];", 'const manifest=await json(config.candidate_catalog.path),checks=[];')
    states = states.replace('for(const id of ["s03_fob_mp-log-trap", "emb05_fob_mp-south-cart"])', 'for(const id of config.entry_ids)')
    states = states.replace("json(join(stage,'library',entry.contract.path))", "json(config.overlay['/library/'+entry.contract.path])")
    states = states.replace("manifest_sha256:sha(await readFile(join(stage,'manifest.json')))", 'manifest_sha256:sha(await readFile(config.candidate_catalog.path))')
    # Pixel digests exercise the actual renderer and prove reset restores exact initial artwork.
    states = states.replace("const canvases=[];", "const digest=async()=>{const p=v.stateDelivery.native.pixels();return Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',p.data)),x=>x.toString(16).padStart(2,'0')).join('')};v.stateDelivery.native.seek(0);v.resetDeliveredState(family.id);const initialDigest=await digest();const canvases=[];")
    states = states.replace("return{checks,canvases}", "const resetDigest=await digest();check('native exact reset pixels',resetDigest===initialDigest);return{checks,canvases,initialDigest,resetDigest,scope:'Raw pre-script initial artwork; conditional Tac19 startup not executed'}")
    states = states.replace("const entry=${JSON.stringify(entry)},contract=", "const expectedPhases=${JSON.stringify(oracle.records.find(r=>r.id===id).phases)},entry=${JSON.stringify(entry)},contract=")
    states = states.replace("const initialDigest=await digest();const canvases=[];", "const initialDigest=await digest();check('native initial reviewed CPU bytes',initialDigest===expectedPhases.find(p=>p.phase==='initial').rgba_sha256);const p=v.stateDelivery.native;for(const f of contract.families){for(const id of f.background_ids??[])p.setBackgroundState(id,'applied',0);for(const id of f.patch_ids??[])p.setPatchState(id,'applied',0);for(const id of f.element_ids??[])p.setElementState(id,true,f.body_terminal_tick)}const appliedDigest=await digest();check('native applied reviewed CPU bytes',appliedDigest===expectedPhases.find(p=>p.phase==='applied').rgba_sha256);v.resetDeliveredState(family.id);const canvases=[];")
    states = states.replace("initialDigest,resetDigest,scope:", "initialDigest,appliedDigest,resetDigest,scope:")
    # Mission controls restore their previous value while async import runs. Do not
    # synthesize a change for the already selected mission or race its replacement.
    states = states.replace("select('Mission',entry.mission);await until(()=>document.querySelector('[aria-label=\"State preview asset\"] option[value=\"'+entry.id+'\"]'));", "if(document.querySelector('select[aria-label=\"Mission\"]').value!==entry.mission){select('Mission',entry.mission);await until(()=>document.querySelector('select[aria-label=\"Mission\"]')?.value===entry.mission&&!document.querySelector('.map-load-dialog'))}await until(()=>document.querySelector('[aria-label=\"State preview asset\"] option[value=\"'+entry.id+'\"]'));")
    states = states.replace("throw Error('State readiness timeout')", "throw Error('State readiness timeout '+JSON.stringify({entry:entry.id,mission:document.querySelector('select[aria-label=\"Mission\"]')?.value,asset:document.querySelector('[aria-label=\"State preview asset\"]')?.value,status:document.querySelector('[aria-label=\"State preview\"] [role=\"status\"]')?.textContent,pageStatus:document.querySelector('#result')?.textContent,dialog:document.querySelector('.map-load-dialog')?.textContent,ready:window.reviewViewport?.stateDelivery.ready,loadedMission:window.reviewViewport?.stateDelivery.contract?.native.mission,loadedFamily:window.reviewViewport?.stateDelivery.contract?.families[0]?.id}))")
    states = states.replace("checks.push(...result.checks);", "const screenshot=await stateProofCommand(ws,nextId,'Page.captureScreenshot',{format:'png',captureBeyondViewport:false},commandTimeoutMs);await writeFile(join(out,id+'-native-reset-ui.png'),Buffer.from(screenshot.data,'base64'));checks.push(...result.checks);")
    assert "if(document.querySelector('select[aria-label=\"Mission\"]').value!==entry.mission)" in states
    (OUT / 'states.mjs').write_text(states)
    run = (OLD / 'run.mjs').read_text().replace(str(OLD / 'states.mjs'), str(OUT / 'states.mjs'))
    start = run.index('const stateRoot=')
    end = run.index('\n\nconst html=', start)
    run = run[:start] + 'const config='+json.dumps(config)+';const served=new Map();const baselineCatalog='+json.dumps(baseline)+';const proposedCatalog='+json.dumps(proposed)+';' + run[end:]
    old = "if(req.url==='/seven-state-proof')"
    new = "const pathname=new URL(req.url,'http://local').pathname;if(Object.hasOwn(config.overlay,pathname)){readFile(join(root,config.overlay[pathname])).then(bytes=>{res.setHeader('Content-Type','application/json');res.end(bytes)}).catch(next);return}if(req.url==='/seven-state-proof')"
    assert old in run
    run = run.replace(old, new)
    start = run.index(' const catalog=await evaluate(')
    end = run.index('\n const stateResult=', start)
    run = run[:start] + ''' const catalog=await evaluate(`(async()=>{const response=await fetch('/library/mission-states/index.json'),bytes=await response.arrayBuffer(),digest=Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',bytes)),x=>x.toString(16).padStart(2,'0')).join(''),actual=JSON.parse(new TextDecoder().decode(bytes)),expected=${JSON.stringify(proposedCatalog)},baseline=${JSON.stringify(baselineCatalog)},ids=${JSON.stringify(config.entry_ids)};if(digest!==${JSON.stringify(config.candidate_catalog.sha256)}||JSON.stringify(actual)!==JSON.stringify(expected)||actual.entries.length!==41)throw Error('Candidate catalog mismatch');const unchanged=baseline.entries.filter(e=>!ids.includes(e.id));if(unchanged.length!==33||!unchanged.every(e=>actual.entries.some(n=>JSON.stringify(n)===JSON.stringify(e))))throw Error('Unchanged33 drift');return{sha256:digest,entries:41,unchanged:33,corrected:8}})()`);await writeFile(join(out,'staged-catalog.json'),JSON.stringify(catalog,null,2));''' + run[end:]
    run = run.replace("const file=join(root,'level-editor/library',relative),expected=sha(await readFile(file));", "const overlayFile=config.overlay['/library/'+relative];const file=overlayFile?join(root,overlayFile):join(root,'level-editor/library',relative),expected=sha(await readFile(file));")
    run = run.replace("selected_entry_ids:[\"s03_fob_mp-log-trap\", \"emb05_fob_mp-south-cart\"]", 'selected_entry_ids:config.entry_ids')
    run = run.replace("status:'PASS_INSTALLED41_NORMAL_HTTP_CHUNK'", "status:'PASS_STAGED_EIGHT_INITIAL_CONTEXTS'")
    run = run.replace("Actual installed41-entry production /library route, no library interception or private state overlay. Exact selected families checked; aggregate must cover all7. All successful normal HTTP resource reads re-fetched and hashed versus unchanged installed files. No publication writes.", "Private nine-path overlay only: proposed41 catalog plus eight corrected contracts. Other resources use normal production route. Exact pre-script initial/reset source pixels and existing physical endpoints checked. Tac19 startup remains actor-conditional. All successful HTTP reads rehashed against exact overlay or unchanged library bytes. No publication writes.")
    run = run.replace("console.log('PASS installed41 chunk '+JSON.stringify([\"s03_fob_mp-log-trap\", \"emb05_fob_mp-south-cart\"]))", "console.log('PASS staged eight initial contexts '+JSON.stringify(config.entry_ids))")
    # Full request traces repeat data already preserved by hashed HTTP receipts.
    run = run.replace("writeFile(join(out,'runtime-events.jsonl'),JSON.stringify(row)+'\\n',{flag:'a'})", "void 0")
    run = run.replace("source_files_unchanged:Object.keys(inputs.files).length,events,elapsed_ms:", "source_files_unchanged:Object.keys(inputs.files).length,event_counts:Object.fromEntries([...new Set(events.map(e=>e.method))].map(method=>[method,events.filter(e=>e.method===method).length])),elapsed_ms:")
    run = run.replace("error:String(error),events,elapsed_ms:", "error:String(error),last_events:events.slice(-50),event_count:events.length,elapsed_ms:")
    (OUT / 'run.mjs').write_text(run)
    for name in ['run.mjs','states.mjs','editor.tsx']:
        p=OUT/name; pins[str(p.relative_to(ROOT))]=sha(p.read_bytes())
    (OUT / 'inputs.json').write_text(json.dumps({'files':pins,'static_map_sha256':pins['level-editor/library/scenes/croisement02.rhlos-map.json']},indent=2)+'\n')
    (OUT / 'runtime-baseline.json').write_text(json.dumps({'source_files':{p:pins[p] for p in sorted(runtime_paths)}},indent=2)+'\n')
    (OUT / 'preparation.json').write_text(json.dumps({'status':'PREPARED_NOT_LAUNCHED','runtime_changes_since_v3':drift,'shared_catalog_delta':catalog_audit,'root_review_required_for_runtime_changes':bool(drift),'overlay_paths':list(config['overlay']),'entries':41,'unchanged_entries':33,'corrected_entries':8,'scope':config['scope']},indent=2)+'\n')
    print(json.dumps({'output':str(OUT.relative_to(ROOT)),'runtime_drift':drift,'pinned_files':len(pins)}))

if __name__=='__main__':
    main()
