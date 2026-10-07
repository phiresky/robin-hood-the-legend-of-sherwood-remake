// Isolated correction of source-backed initial context; never writes the installed library.
import assert from 'node:assert/strict';
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {resolve,join,dirname} from 'node:path';
import {createHash} from 'node:crypto';
import {addInitialContexts} from './restart16_initial_context.mjs';
import {verifyNativePresentationSource} from '../../app/src/native-state-presentation.ts';
const root=resolve('level-editor/work/croisement02-refinement');
const library=resolve('level-editor/library');
const prior=join(root,'restart7-source-patch-delivery/contracts-v1');
const output=join(root,'restart2-state/restart16-initial-context-v1');
const read=async p=>JSON.parse(await readFile(p,'utf8'));
const sha=b=>createHash('sha256').update(b).digest('hex');
const expected=new Map([['Tac06_FoB_EC',[0,1,2,3,4,5,6,7]],['Tac19_FoB_EC',[0,15]]]);
const indexPath=join(library,'mission-states/index.json');
const indexBytes=await readFile(indexPath),index=JSON.parse(indexBytes);
assert.equal(index.entries.length,41);
const acceptancePath=join(root,'restart2-state/installed41-normal-http-final-v1/root-acceptance.json');
const acceptance=await read(acceptancePath);
assert.equal(sha(indexBytes),acceptance.catalog_sha256);
const preparation=await read(join(prior,'manifest.json'));
const resourceSources=new Map(preparation.resources.map(r=>[r.path,r]));
const level=await read(join(library,'game-data/Data/Levels/Croisement02.rhp.json'));
const resources=new Map(),files=[],records=[];
for(const [mission,indices] of expected){
  const data=await read(join(library,`game-data/Data/Levels/${mission}.rhm.json`));
  const additions=[];
  for(const sourceIndex of indices){
    const id=`mission-${mission}-patch-${String(sourceIndex).padStart(3,'0')}`;
    const prepared=preparation.records.find(r=>r.id===id);
    assert.equal(prepared?.status,'SOURCE_BINDINGS_PASS');
    const bytes=await readFile(join(prior,prepared.contract));
    assert.equal(sha(bytes),prepared.sha256);
    const template=JSON.parse(bytes).native.patch_states.find(s=>s.id===id);
    assert.equal(template.source.index,sourceIndex);
    const raw=data.mission_patches[sourceIndex];
    assert.equal(raw.element_fx.active,true);
    assert.equal(raw.start_animation_valid,true);
    assert.equal(template.source.sha256,sha(JSON.stringify(raw)));
    additions.push(template);
    for(const resource of [template.profile,...template.initial,...template.transition,...template.final]){
      const source=resourceSources.get(resource.path);
      assert.ok(source,`Missing frozen resource ${resource.path}`);
      assert.equal(source.sha256,resource.sha256);
      resources.set(resource.path,{...resource,source:source.source});
    }
  }
  for(const entry of index.entries.filter(e=>e.mission===mission)){
    const bytes=await readFile(join(library,entry.contract.path));
    assert.equal(sha(bytes),entry.contract.sha256);
    const original=JSON.parse(bytes),contract=addInitialContexts(original,additions);
    assert.deepEqual(contract.families,original.families);
    const source={name:mission,data,level,camera:{kind:'oblique-orthographic',elevation_deg:35}};
    await verifyNativePresentationSource(contract.native,source);
    const encoded=Buffer.from(JSON.stringify(contract)+'\n');
    files.push({path:entry.contract.path,bytes:encoded});
    records.push({id:entry.id,mission,path:entry.contract.path,baseline_sha256:sha(bytes),sha256:sha(encoded),added_context_ids:additions.map(s=>s.id),families_sha256:sha(JSON.stringify(contract.families))});
  }
}
const reused=[];
for(const [path,resource] of resources){
  const bytes=await readFile(resource.source);
  assert.equal(sha(bytes),resource.sha256);
  let present;try{present=await readFile(join(library,path));}catch(e){if(e.code!=='ENOENT')throw e;}
  if(present){assert.equal(sha(present),resource.sha256);reused.push({path,sha256:resource.sha256});}
  else files.push({path,bytes});
}
const payloadBytes=files.reduce((sum,f)=>sum+f.bytes.length,0);
assert.ok(payloadBytes<6*1024*1024,'Leave at least2MiB for bounded verification evidence');
await mkdir(output,{recursive:false});
for(const file of files){const p=join(output,'library',file.path);await mkdir(dirname(p),{recursive:true});await writeFile(p,file.bytes,{flag:'wx'});}
assert.equal(sha(await readFile(indexPath)),sha(indexBytes));
for(const r of records)assert.equal(sha(await readFile(join(library,r.path))),r.baseline_sha256);
const result={status:'PRIVATE_INITIAL_CONTEXT_CANDIDATE; CPU and root review pending',catalog_entries:41,catalog_sha256:sha(indexBytes),acceptance_sha256:sha(await readFile(acceptancePath)),unique_added_patch_instances:10,affected_contracts:records.length,payload_bytes:payloadBytes,records,files:files.map(f=>({path:f.path,sha256:sha(f.bytes),bytes:f.bytes.length})),reused,scope:'Existing controls, physical models, catalog entries and canonical contracts unchanged; no publication or gameplay claim.'};
await writeFile(join(output,'manifest.json'),JSON.stringify(result,null,2)+'\n',{flag:'wx'});
console.log(JSON.stringify({status:result.status,contracts:records.length,unique_patches:10,payload_bytes:payloadBytes,new_files:files.length,reused_resources:reused.length}));
