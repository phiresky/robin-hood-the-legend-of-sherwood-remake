import assert from 'node:assert/strict';
import {readFile,writeFile} from 'node:fs/promises';
import {resolve,join} from 'node:path';
import {createHash} from 'node:crypto';
import {parseStoredMap} from '../../shared/src/stored-level.ts';
import {loadMissionStatePreview,parseMissionStateCatalog} from '../../app/src/mission-state-catalog.ts';
import {verifyStateApertureReceivers} from '../../app/src/state-aperture-loader.ts';
const base=resolve('level-editor/work/active/croisement02/restart53-hole-mound-transaction-v1'),lib=resolve('level-editor/library'),stage=join(base,'library');
const sha=b=>createHash('sha256').update(b).digest('hex');
const transaction=JSON.parse(await readFile(join(base,'transaction.json')));
async function checked(p){const raw=await readFile(p.path);assert.equal(sha(raw),p.sha256,p.path);return raw}
for(const p of [transaction.catalog_before,transaction.map_source_guard,...transaction.approvals,...transaction.browser_proofs,...transaction.receiver_source_guards.flatMap(r=>[r.model,r.descriptor])])await checked(p);
const files=new Map(transaction.files.map(r=>[r.destination,r]));for(const row of files.values())await checked(row.staged);
const saved=JSON.parse(await checked(transaction.map_source_guard)),descriptors=new Map();for(const r of [...saved.assetSources??[],...saved.sceneAssets??[]]){const raw=await readFile(join(lib,r.descriptor));assert.equal(sha(raw),r.descriptor_sha256);descriptors.set(r.id,JSON.parse(raw));}
const document=parseStoredMap(saved,descriptors);
function directory(prefix=''){return{async getDirectoryHandle(name){return directory(prefix+name+'/')},async getFileHandle(name){return{async getFile(){const logical=prefix+name,row=files.get(logical);const raw=row?await checked(row.staged):await readFile(join(lib,logical));return new File([raw],name)}}}}}
const catalog=parseMissionStateCatalog(JSON.parse(await readFile(join(stage,'mission-states/index.json'))));const before=JSON.parse(await checked(transaction.catalog_before));assert.deepEqual(catalog.slice(0,before.entries.length),before.entries);
const results=[];
for(const entry of transaction.catalog_additions){const loaded=await loadMissionStatePreview(directory(),entry);assert.equal(loaded.kind,'transition');if(loaded.aperture)verifyStateApertureReceivers(loaded.aperture,document);for(const family of loaded.contract.families)for(const phase of ['initial','applied'])if(Array.isArray(family.physical[phase]))for(const binding of family.physical[phase]){const row=files.get(binding.model);assert.ok(row);assert.equal(row.staged.sha256,binding.model_sha256)}results.push({id:entry.id,aperture:!!loaded.aperture,production_catalog_source_and_receivers_valid:true})}
assert.equal(results.length,62);assert.equal(results.filter(r=>r.aperture).length,30);
const result={status:'PASS_PRIVATE_PRODUCTION_CATALOG_TRANSACTION',entries:results,unrelated_catalog_entries_preserved:before.entries.length,model_files:transaction.files.filter(r=>r.destination.endsWith('.glb')).length,canonical_writes:false,browser_wiring_verified:false};await writeFile(join(base,'verification.json'),JSON.stringify(result,null,2)+'\n');console.log(result.status,results.length);
