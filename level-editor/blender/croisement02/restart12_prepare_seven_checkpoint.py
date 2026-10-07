"""Prepare bounded state checks against an existing immutable HTTP/runtime snapshot."""
from pathlib import Path
import hashlib,json,sys
ROOT=Path(__file__).resolve().parents[3]
old=Path(sys.argv[1]).resolve();b=old.with_name(sys.argv[2]);ids=sys.argv[3:];assert ids and not b.exists()
b.mkdir();(b/'contacts').mkdir()
files=['editor.tsx','run.mjs','states.mjs','capture.mjs','contacts/proof.ts','contacts/index.html','contacts/manifest.json','runtime-baseline.json']
for n in files:
 s=(old/n).read_text()
 for name in files:s=s.replace(str(old/name),str(b/name))
 (b/n).write_text(s)
(b/'node_modules').symlink_to((ROOT/'level-editor/app/node_modules').resolve(),target_is_directory=True)
prior=old/'browser/remaining-seven-states/early-visual-gate.json';assert json.loads(prior.read_text())['status']=='PASS'
p=b/'states.mjs';s=p.read_text();s=s.replace('const manifest=await json(join(stage,\'manifest.json\')),checks=[],screenshots=[];',"const manifest=await json(join(stage,'manifest.json')),checks=[],screenshots=[];const selectedIds="+json.dumps(ids)+";if(selectedIds.some(id=>!manifest.entries.some(e=>e.id===id)))throw Error('Unknown family');")
s=s.replace("checks.push({name,pass});", "checks.push({name,pass});await writeFile(join(out,'assertion-ledger.json'),JSON.stringify({checks},null,2));")
s=s.replace('for(const entry of manifest.entries){','for(const entry of manifest.entries.filter(e=>selectedIds.includes(e.id))){const firstCheck=checks.length,firstImage=screenshots.length;')
a=s.index(' if(entry.id===manifest.entries[0].id)');z=s.index('\n\n }',a)
s=s[:a]+" const familyImages=[];for(const name of screenshots.slice(firstImage)){familyImages.push({path:name,sha256:sha(await readFile(join(out,name))),guard_sha256:sha(await readFile(join(out,name.replace(/\\.png$/,'-canvas-guard.json'))))})}await writeFile(join(out,entry.id+'-terminal.json'),JSON.stringify({status:'PASS',entry_id:entry.id,contract_sha256:sha(await readFile(join(stage,'library',entry.contract.path))),checks:checks.slice(firstCheck),images:familyImages,scope:'Exact family assertions and all three display states completed; snapshot postflight binding remains required.'},null,2));"+s[z:]
s=s.replace("const earlyGateSha=sha(await readFile(join(out,'early-visual-gate.json')))","const earlyGateSha=sha(await readFile("+json.dumps(str(prior))+"))")
s=s.replace("scope:'Seven private controlled previews", "selected_entry_ids:selectedIds,scope:'Bounded subset of seven private controlled previews")
p.write_text(s)
p=b/'run.mjs';s=p.read_text();a=s.index(" const contacts=await import(");z=s.index(' await verifyPins();',a);s=s[:a]+" const contactResult={status:'DEFERRED_TO_SEPARATE_FULL_CONTACT_PROOF',views:[]};\n"+s[z:];s=s.replace("status:'PASS',inputs_sha256","status:'PASS_SCOPED_STATE_CHUNK',selected_entry_ids:"+json.dumps(ids)+",inputs_sha256");s=s.replace("console.log('PASS seven states and sixteen contact views')","console.log('PASS scoped state chunk '+JSON.stringify("+json.dumps(ids)+"))");p.write_text(s)
h=lambda p:hashlib.sha256(p.read_bytes()).hexdigest();d=json.loads((old/'inputs.json').read_text())
for p in b.rglob('*'):
 if p.is_file()and 'node_modules'not in p.parts:d['files'][str(p.relative_to(ROOT))]=h(p)
d['files'][str(prior.relative_to(ROOT))]=h(prior);d['files'][str(Path(__file__).relative_to(ROOT))]=h(Path(__file__));d['scope']='Bounded state chunk over unchanged frozen library/runtime; all family checks preserved. Aggregate must cover exact seven families/97checks/21images and separately16contacts.';(b/'inputs.json').write_text(json.dumps(d,indent=2)+'\n');print(b)
