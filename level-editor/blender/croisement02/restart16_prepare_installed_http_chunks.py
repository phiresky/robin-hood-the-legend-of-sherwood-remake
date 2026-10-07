from pathlib import Path
import hashlib,json
R=Path(__file__).resolve().parents[3];S=R/'level-editor/work/croisement02-refinement/restart2-state';old=S/'current-live-seven-smoke-v5';h=lambda p:hashlib.sha256(p.read_bytes()).hexdigest();manifest=json.loads((S/'remaining-seven-package-v2/manifest.json').read_text());plan=json.loads((S/'remaining-seven-publication-v1/plan.json').read_text());expected='135e7e443ede18656309245a0609ee537f3d14f929baf303490099d8af177825';live=R/'level-editor/library';assert h(live/'mission-states/index.json')==expected
before=json.loads((S/'remaining-seven-publication-v1/installed34-index.json').read_text())['entries'];after=json.loads((live/'mission-states/index.json').read_text())['entries'];assert len(after)==41 and all(e in after for e in before)
chunks=[('installed41-normal-http-chunk-1-v1',['s03_fob_mp-log-trap','emb05_fob_mp-south-cart']),('installed41-normal-http-remaining5-v1',['tac19_fob_ec-south-cart','emb09_fob_jms-north-cart','emb05_fob_mp-south-field-fence','tac02_fob_ec-south-field-fence','tac19_fob_ec-south-field-fence'])]
for name,ids in chunks:
 b=S/name;b.mkdir()
 for n in ['editor.tsx','run.mjs','states.mjs','runtime-baseline.json']:(b/n).write_text((old/n).read_text().replace(str(old),str(b)).replace('/seven-library/','/library/'))
 (b/'node_modules').symlink_to((R/'level-editor/app/node_modules').resolve(),target_is_directory=True)
 p=b/'states.mjs';s=p.read_text().replace("['emb05_fob_mp-south-field-fence']",json.dumps(ids));s=s.replace("const result=await evaluate(`(async()=>{const entry=", "const result=await evaluate(`(async()=>{const entry=")
 # Record each family independently after complete action checks; source full97 proof remains separate.
 p.write_text(s)
 p=b/'run.mjs';s=p.read_text();a=s.index('plugins:[{');z=s.index('],server:',a);s=s[:a]+"plugins:[{name:'installed41-fixture-page-only',enforce:'pre',configureServer(s){s.middlewares.use((req,res,next)=>{if(req.url==='/seven-state-proof'){res.setHeader('Content-Type','text/html');res.end(html);return}next()})}}"+s[z:]
 s=s.replace("['Runtime.exceptionThrown','Inspector.detached','Network.loadingFailed','Network.requestWillBeSent','Network.responseReceived']", "['Runtime.exceptionThrown','Runtime.consoleAPICalled','Inspector.detached','Network.loadingFailed','Network.requestWillBeSent','Network.responseReceived']")
 marker=' await verifyPins();for(const[path,expected]of served)'
 extra=r''' const responseUrls=[...new Set(events.filter(e=>e.method==='Network.responseReceived'&&e.params.response.status===200&&e.params.response.url.startsWith(origin+'/library/')).map(e=>e.params.response.url))];const httpPins=[];for(const url of responseUrls){const relative=decodeURIComponent(new URL(url).pathname.slice('/library/'.length));const response=await fetch(url);if(!response.ok)throw Error('Normal HTTP postflight failed '+relative);const hash=createHash('sha256');for await(const bytes of response.body)hash.update(bytes);const actual=hash.digest('hex');if(relative==='scenes/index.json'){httpPins.push({path:relative,sha256:actual,virtualProductionListing:true});continue}const file=join(root,'level-editor/library',relative),expected=sha(await readFile(file));if(actual!==expected)throw Error('Normal HTTP/disk bytes differ '+relative);served.set(file,expected);httpPins.push({path:relative,sha256:actual})}await writeFile(join(out,'normal-http-resources.json'),JSON.stringify(httpPins,null,2));
'''
 assert marker in s;s=s.replace(marker,extra+marker)
 s=s.replace("e.method==='Runtime.exceptionThrown'||", "e.method==='Runtime.exceptionThrown'||(e.method==='Runtime.consoleAPICalled'&&e.params.type==='error')||")
 s=s.replace("status:'PASS_CURRENT_LIVE_SCOPED_SMOKE'", "status:'PASS_INSTALLED41_NORMAL_HTTP_CHUNK'").replace('selected_entry_ids:["emb05_fob_mp-south-field-fence"]','selected_entry_ids:'+json.dumps(ids));s=s.replace("console.log('PASS current-live Emb05 fence smoke')", "console.log('PASS installed41 chunk '+JSON.stringify("+json.dumps(ids)+"))")
 s=s.replace("scope:'Current live runtime and current map via production HTTP adapter with exact private seven-package overlay. Bounded state smoke only. Favicon404 and expected AVIF probes with successful WebP fallback are disclosed in network events. No publication.'", "scope:'Actual installed41-entry production /library route, no library interception or private state overlay. Exact selected families checked; aggregate must cover all7. All successful normal HTTP resource reads re-fetched and hashed versus unchanged installed files. No publication writes.'")
 catalog_code=" const catalog=await evaluate(`(async()=>{const response=await fetch('/library/mission-states/index.json'),bytes=await response.arrayBuffer(),digest=Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',bytes)),x=>x.toString(16).padStart(2,'0')).join(''),entries=JSON.parse(new TextDecoder().decode(bytes)).entries,before="+json.dumps(before)+";if(digest!=="+json.dumps(expected)+"||entries.length!==41||!before.every(old=>entries.some(now=>JSON.stringify(now)===JSON.stringify(old))))throw Error('Installed41/old34 catalog mismatch');return{sha256:digest,entries:41,old34unchanged:true}})()`);await writeFile(join(out,'installed-catalog.json'),JSON.stringify(catalog,null,2));\n"
 s=s.replace(' const stateResult=await states(',catalog_code+' const stateResult=await states(')
 p.write_text(s)
 d=json.loads((old/'inputs.json').read_text());d['files']={k:v for k,v in d['files'].items() if not k.startswith(str(old.relative_to(R))+'/')}
 for p in b.iterdir():
  if p.is_file():d['files'][str(p.relative_to(R))]=h(p)
 for path,digest in plan['baseline_files'].items():
  if path=='mission-states/index.json':continue
  p=live/path;assert h(p)==digest;d['files'][str(p.relative_to(R))]=digest
 for row in manifest['files']+manifest['reused']:
  p=live/row['path'];assert h(p)==row['sha256'];d['files'][str(p.relative_to(R))]=row['sha256']
 d['files'][str((live/'mission-states/index.json').relative_to(R))]=expected
 for n in ['final-current-publication-gate-v1/installation.json','remaining-seven-publication-v1/installed34-index.json']:
  p=S/n;d['files'][str(p.relative_to(R))]=h(p)
 for row in json.loads((S/'final-current-normal-http-v2/verification.json').read_text())['checks']:
  p=live/row['path'];assert h(p)==row['sha256'];d['files'][str(p.relative_to(R))]=row['sha256']
 d['installed_catalog']={'sha256':expected,'entries':41,'original_entries_preserved':34,'added_ids':plan['added_ids']};d['scope']='Actual normal-library installed41 proof, no private state overlay.';(b/'inputs.json').write_text(json.dumps(d,indent=2))
 print(b,flush=True)
