import assert from 'node:assert/strict';
import {readFile,writeFile} from 'node:fs/promises';
import {resolve,join} from 'node:path';
import {createHash} from 'node:crypto';
import {loadMissionStatePreview,parseMissionStateCatalog} from '../../app/src/mission-state-catalog.ts';
const base=resolve('level-editor/work/active/croisement02/restart59-climbing-catalog-stage-v1'),lib=resolve('level-editor/library');
const sha=b=>createHash('sha256').update(b).digest('hex');
const plan=JSON.parse(await readFile(join(base,'plan.json'))),files=new Map(plan.files.map(r=>[r.destination,r]));
async function checked(p){const raw=await readFile(p.path);assert.equal(sha(raw),p.sha256);return raw}
for(const p of [plan.approval,plan.endpoint_browser,plan.source_config])await checked(p);
for(const row of plan.files)await checked(row.staged);
function directory(prefix=''){return{async getDirectoryHandle(name){return directory(prefix+name+'/')},async getFileHandle(name){return{async getFile(){const logical=prefix+name,row=files.get(logical),raw=row?await checked(row.staged):await readFile(join(lib,logical));return new File([raw],name)}}}}}
const catalog=parseMissionStateCatalog(JSON.parse(await checked(files.get('mission-states/index.json').staged)));
const results=[];
for(const entry of plan.catalog_additions){assert.deepEqual(catalog.find(e=>e.id===entry.id),entry);const loaded=await loadMissionStatePreview(directory(),entry);assert.equal(loaded.kind,'transition');assert.equal(loaded.aperture,undefined);assert.equal(loaded.contract.families.length,1);const family=loaded.contract.families[0];assert.equal(family.body_terminal_tick,5);for(const phase of ['initial','applied'])for(const binding of family.physical[phase])assert.equal(files.get(binding.model).staged.sha256,binding.model_sha256);results.push({id:entry.id,production_source_verified:true});}
assert.equal(results.length,2);const report={status:'PASS_PRIVATE_CLIMBING_CATALOG',controls:results,canonical_writes:false};await writeFile(join(base,'verification.json'),JSON.stringify(report,null,2)+'\n');console.log(report.status);
