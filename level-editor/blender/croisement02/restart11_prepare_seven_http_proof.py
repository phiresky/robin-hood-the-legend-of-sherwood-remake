"""Prepare the seven-state check with the production HTTP library adapter."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[3]
WORK = ROOT / 'level-editor/work/croisement02-refinement'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    expected, name = sys.argv[1:]
    subprocess.run([sys.executable, str(Path(__file__).with_name('restart10_prepare_seven_state_proof.py')),
                    expected, name], check=True)
    out = WORK / 'restart2-state' / name
    fixture = (WORK / 'restart2-textures/installed-static-v2/editor-review.tsx').read_text()
    fixture = fixture.replace("const root = '/library/';", "const root = '/seven-library/';")
    fixture = fixture.replace('await openHttpLibrary()', "await openHttpLibrary('/seven-library/')")
    (out / 'editor.tsx').write_text(fixture)
    source = Path(__file__).with_name('restart10_verify_seven_state_editor.mjs')
    runner = source.read_text()
    runner = runner.replace("'../../app/tests/cdp.mjs'", json.dumps((ROOT / 'level-editor/app/tests/cdp.mjs').as_uri()))
    runner = runner.replace("'./restart6_verify_remaining_state_editor.mjs'", json.dumps(source.with_name('restart6_verify_remaining_state_editor.mjs').as_uri()))
    start = runner.index("const {createServer}=")
    end = runner.index('await server.listen();', start)
    replacement = r'''const {createServer}=await import(VITE_MODULE);
const stateRoot=join(root,'level-editor/work/croisement02-refinement/restart2-state/remaining-seven-package-v2/library');
const manifest=JSON.parse(await readFile(join(stateRoot,'../manifest.json'),'utf8'));
const overlay=new Set(manifest.files.map(row=>row.path));overlay.add('mission-states/index.json');
const html='<html><body style="margin:0"><div id="result">STARTING</div><div id="root"></div><script type="module" src="/@fs'+join(base,'editor.tsx')+'"></script></body></html>';
const server=await createServer({configFile:join(root,'level-editor/app/vite.config.ts'),root:join(root,'level-editor/app'),cacheDir:join(out,'vite-cache'),plugins:[{name:'seven-state-http-overlay',enforce:'pre',configureServer(s){s.middlewares.use((req,res,next)=>{
 if(req.url==='/seven-state-proof'){res.setHeader('Content-Type','text/html');res.end(html);return}
 if(!req.url?.startsWith('/seven-library/'))return next();
 const relative=decodeURIComponent(req.url.split('?')[0].slice('/seven-library/'.length));
 if(relative.split('/').some(p=>!p||p.startsWith('.')||p.includes('\\'))){res.writeHead(400).end();return}
 if(!overlay.has(relative)){res.writeHead(307,{Location:'/library/'+relative}).end();return}
 readFile(join(stateRoot,relative)).then(bytes=>{res.setHeader('Content-Type',relative.endsWith('.json')?'application/json':relative.endsWith('.png')?'image/png':'model/gltf-binary');res.end(bytes)}).catch(next);
 })}}],server:{host:'127.0.0.1',port:0,watch:null,hmr:false}});
'''.replace('VITE_MODULE', json.dumps((ROOT / 'level-editor/app/node_modules/vite/dist/node/index.js').as_uri()))
    runner = runner[:start] + replacement + runner[end:]
    old = "const status=await evaluate('document.querySelector(\"#result\")?.textContent');"
    new = "const progress=await evaluate('({status:document.querySelector(\"#result\")?.textContent,title:document.title,readyState:document.readyState,rootBytes:document.querySelector(\"#root\")?.innerHTML.length,viewport:!!window.reviewViewport})');await writeFile(join(out,'progress.json'),JSON.stringify({elapsed_ms:Date.now()-started,...progress},null,2));const status=progress.status;"
    assert old in runner
    runner = runner.replace(old, new)
    (out / 'run.mjs').write_text(runner)
    inputs = json.loads((out / 'inputs.json').read_text())
    for path in (out / 'editor.tsx', out / 'run.mjs', Path(__file__), ROOT / 'level-editor/app/vite.config.ts'):
        inputs['files'][str(path.relative_to(ROOT))] = sha(path)
    inputs['scope'] = 'Production HTTP library adapter with seven exact private state entries; installed static map unchanged.'
    (out / 'inputs.json').write_text(json.dumps(inputs, indent=2) + '\n')
    print(out / 'run.mjs')


if __name__ == '__main__':
    main()
