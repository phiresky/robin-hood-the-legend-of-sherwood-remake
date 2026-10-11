/** Freeze current sources and actual installed canonical state resources; no overlays. */
import {readFile,writeFile,mkdir,readdir,copyFile} from 'node:fs/promises';
import {resolve,join,dirname} from 'node:path';
import {createHash} from 'node:crypto';
const root=resolve('.'),old='level-editor/work/croisement02-refinement/restart63-hole-mound-current-runtime-browser-v1',out='level-editor/work/croisement02-refinement/restart73-hole-mound-canonical-browser-v1';
const hash=bytes=>createHash('sha256').update(bytes).digest('hex');const read=async p=>JSON.parse(await readFile(p));const pin=async p=>({path:resolve(p),sha256:hash(await readFile(p))});const save=async(p,d)=>writeFile(p,JSON.stringify(d,null,2)+'\n',{flag:'wx'});
await mkdir(out);
for(const name of ['editor.tsx','check-config.json','check-v3.js'])await copyFile(join(old,name),join(out,name));
let run=await readFile(join(old,'run-v3.mjs'),'utf8');run=run.replaceAll(old,out).replaceAll(out+'/browser-v3',out+'/browser-v1');await writeFile(join(out,'run-v3.mjs'),run,{flag:'wx'});
const authority=await read(join(old,'runtime-authority.json'));
const sourceFiles=async dir=>{const rows=[];for(const e of await readdir(dir,{withFileTypes:true})){const p=join(dir,e.name);if(e.isDirectory())rows.push(...await sourceFiles(p));else if(e.isFile()&&!/[.]test[.](ts|mjs|js)$/.test(p))rows.push(p)}return rows.sort()};
const snapshot={};for(const p of(await Promise.all(authority.sourceDirectories.map(sourceFiles))).flat().sort()){const bytes=await readFile(p);snapshot[p]=hash(bytes);if(/\.(?:tsx?|jsx?|mjs|cjs|css|json)$/.test(p)){const target=join(out,'snapshot',p);await mkdir(dirname(target),{recursive:true});await writeFile(target,bytes,{flag:'wx'})}}
if(!snapshot['level-editor/shared/src/reviewed-node-poses.ts'])throw Error('Current node pose helper missing');
await save(join(out,'source-snapshot.json'),snapshot);
authority.membership=Object.keys(snapshot).sort();authority.status='CANONICAL_POSTINSTALL_HOLE_MOUND_RUNTIME_FROZEN';authority.runtime_commit='ee3694cd4';
const indexBytes=await readFile(authority.index.path),index=JSON.parse(indexBytes);const stable=v=>Array.isArray(v)?v.map(stable):v&&typeof v==='object'?Object.fromEntries(Object.keys(v).sort().map(k=>[k,stable(v[k])])):v;const subset=index.assets.filter(a=>a.source_map?.toLowerCase()==='croisement02').sort((a,b)=>a.id<b.id?-1:a.id>b.id?1:0);authority.index.initial_sha256=hash(indexBytes);authority.index.c02_sha256=hash(JSON.stringify(stable(subset)));authority.index.c02_count=subset.length;await save(join(out,'runtime-authority.json'),authority);
const runtime={};for(const p of Object.keys(await read(join(old,'runtime-inputs.json'))))runtime[p]=snapshot[p]??hash(await readFile(p));Object.assign(runtime,snapshot);await save(join(out,'runtime-inputs.json'),runtime);
const installed='level-editor/work/croisement02-refinement/restart58-hole-mound-publication-v1/root-scoped-apply-v3';
const plan=await read(join(installed,'plan.json')),journal=await read(join(installed,'transaction/journal.json'));
if(journal.phase!=='INSTALLED_LIVE_BROWSER_PENDING')throw Error('Installed transaction incomplete');
for(const row of plan.records)if(hash(await readFile(row.target))!==row.new_sha256)throw Error('Installed target drift '+row.target);
const current=await read('level-editor/library/mission-states/index.json');
if(current.entries.length!==103||JSON.stringify(current.entries)!==JSON.stringify([...plan.catalog_before.entries,...plan.additions]))throw Error('Canonical103 catalog differs');
const overlay={status:'CANONICAL_POSTINSTALL_NO_OVERLAYS',overlay_routes:{},installed_plan:await pin(join(installed,'plan.json')),installed_journal:await pin(join(installed,'transaction/journal.json')),scope:'Actual canonical103-entry catalog and89 installed resources. Representative controls/edit guard; no gameplay compiler claim.'};
await save(join(out,'overlay-request.json'),overlay);
const inputs={};for(const name of ['editor.tsx','check-config.json','check-v3.js','run-v3.mjs','runtime-authority.json','runtime-inputs.json','source-snapshot.json','overlay-request.json'])inputs[join(out,name)]=hash(await readFile(join(out,name)));for(const row of plan.records)inputs[row.target]=row.new_sha256;for(const p of [overlay.installed_plan,overlay.installed_journal,plan.root_scope_receipt,plan.scoped_runtime_authority])inputs[p.path]=p.sha256;
const prior=join(old,'root-review-ready.json');inputs[prior]=hash(await readFile(prior));await save(join(out,'inputs-v3.json'),inputs);
const result={status:'PREPARED_NOT_EXECUTED',runner:await pin(join(out,'run-v3.mjs')),source_snapshot:await pin(join(out,'source-snapshot.json')),inputs:await pin(join(out,'inputs-v3.json')),source_files:Object.keys(snapshot).length,overlay_routes:Object.keys(overlay.overlay_routes).length,current_catalog_entries_preserved:current.entries.length,prior_exhaustive_scope:await pin(prior),browser_started:false,canonical_writes:false};await save(join(out,'preparation.json'),result);console.log(JSON.stringify(result));
