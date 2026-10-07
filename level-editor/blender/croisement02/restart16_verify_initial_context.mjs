// Exercise real native decoding, ordered composition and existing controls on private contracts.
import assert from 'node:assert/strict';
import {readFile,writeFile} from 'node:fs/promises';
import {resolve,join} from 'node:path';
import {createHash} from 'node:crypto';
import {NativeStatePresentation,verifyNativePresentationSource,decodeNativeResource} from '../../app/src/native-state-presentation.ts';
import {nativeTransientPatchFrame} from '../../shared/src/native-state-presentation.ts';
const library=resolve('level-editor/library'),stage=resolve('level-editor/work/croisement02-refinement/restart2-state/restart16-initial-context-v1');
const read=async p=>JSON.parse(await readFile(p,'utf8'));
const sha=x=>createHash('sha256').update(x).digest('hex');
const manifest=await read(join(stage,'manifest.json'));
const resource=async r=>{let b;try{b=await readFile(join(stage,'library',r.path));}catch(e){if(e.code!=='ENOENT')throw e;b=await readFile(join(library,r.path));}assert.equal(sha(b),r.sha256);return new Uint8Array(b);};
const level=await read(join(library,'game-data/Data/Levels/Croisement02.rhp.json'));
const records=[],negative=[];
for(const row of manifest.records){
 const baseBytes=await readFile(join(library,row.path));assert.equal(sha(baseBytes),row.baseline_sha256);
 const bytes=await readFile(join(stage,'library',row.path));assert.equal(sha(bytes),row.sha256);
 const base=JSON.parse(baseBytes),contract=JSON.parse(bytes),source={name:row.mission,data:await read(join(library,`game-data/Data/Levels/${row.mission}.rhm.json`)),level,camera:{kind:'oblique-orthographic',elevation_deg:35}};
 assert.deepEqual(contract.families,base.families);
 const additions=contract.native.patch_states.filter(s=>row.added_context_ids.includes(s.id));
 const restored=structuredClone(contract);restored.native.patch_states=restored.native.patch_states.filter(s=>!row.added_context_ids.includes(s.id));if(!('patch_states'in base.native))delete restored.native.patch_states;
 assert.deepEqual(restored,base,'Only listed context additions may change');
 const boundIds=new Set(contract.families.flatMap(f=>[...(f.element_ids??[]),...(f.background_ids??[]),...(f.patch_ids??[])]));
 const frames=[];
 for(const state of additions){
  assert.ok(!boundIds.has(state.id),'New context cannot become a control');
  if(state.integrate_in_background)assert.equal(state.activation,'initial-only');
  assert.equal(state.initial.length,1);
  for(const tick of [0,1,10000])assert.deepEqual(nativeTransientPatchFrame(state,'initial',tick),state.initial[0]);
  const decoded=await decodeNativeResource(state.initial[0],resource);let opaque=0,blue=0;
  for(let i=0;i<decoded.data.length;i+=4)if(decoded.data[i+3]){opaque++;if(decoded.data[i]===0&&decoded.data[i+1]===0&&decoded.data[i+2]===255)blue++;}
  assert.equal(blue,0,'Initial artwork cannot silently paint a shadow key blue');
  frames.push({id:state.id,opaque_pixels:opaque,elevation:state.elevation,layer:state.layer,source_index:state.source.index});
 }
 const a=new NativeStatePresentation(),b=new NativeStatePresentation();
 try{
  assert.equal(await a.set(base.native,source,resource),true);assert.equal(await b.set(contract.native,source,resource),true);
  const phases=[];
  for(const phase of ['initial','applied','reset']){
   for(const p of[a,b]){
    p.seek(0);
    for(const family of contract.families){
     const state=phase==='applied'?'applied':'initial';
     for(const id of family.background_ids??[])p.setBackgroundState(id,state,0);
     for(const id of family.patch_ids??[])p.setPatchState(id,state,0);
     for(const id of family.element_ids??[]){const original=base.native.elements.find(e=>e.id===id);p.setElementState(id,phase==='applied'?true:original.active,phase==='applied'?family.body_terminal_tick:0);}
    }
   }
   const before=a.pixels(),after=b.pixels();assert.equal(before.width,after.width);assert.equal(before.height,after.height);
   let different=0,outside=0;
   for(let i=0;i<before.data.length;i+=4){if(before.data[i]===after.data[i]&&before.data[i+1]===after.data[i+1]&&before.data[i+2]===after.data[i+2]&&before.data[i+3]===after.data[i+3])continue;different++;const pixel=i/4,x=pixel%before.width+contract.native.origin[0],y=Math.floor(pixel/before.width)+contract.native.origin[1];const within=additions.some(s=>s.initial.some(f=>x>=s.display_position[0]+f.offset[0]&&x<s.display_position[0]+f.offset[0]+f.width&&y>=s.display_position[1]+f.offset[1]&&y<s.display_position[1]+f.offset[1]+f.height));if(!within)outside++;}
   assert.equal(outside,0,'Existing pixels outside missing context bounds must survive');
   phases.push({phase,changed_pixels:different,outside_context_changed:outside,rgba_sha256:sha(after.data)});
  }
  assert.equal(phases[0].rgba_sha256,phases[2].rgba_sha256,'Reset must recover exact corrected initial appearance');
  records.push({id:row.id,status:'PASS',added:frames,phases});
 }finally{a.dispose();b.dispose();}
 if(row.id==='tac06_fob_ec-log-trap'){
  for(const field of ['source','position','elevation','creation']){
   const changed=structuredClone(contract.native),s=changed.patch_states.find(s=>row.added_context_ids.includes(s.id));
   if(field==='source')s.source.sha256='0'.repeat(64);
   if(field==='position')s.display_position[0]++;
   if(field==='elevation')s.elevation++;
   if(field==='creation')s.creation_order++;
   await assert.rejects(()=>verifyNativePresentationSource(changed,source));negative.push({mutation:field,rejected:true});
  }
 }
}
assert.equal(sha(await readFile(join(library,'mission-states/index.json'))),manifest.catalog_sha256);
for(const row of manifest.records)assert.equal(sha(await readFile(join(library,row.path))),row.baseline_sha256);
const result={status:'PASS_PRIVATE_CPU_INITIAL_CONTEXT',manifest_sha256:sha(await readFile(join(stage,'manifest.json'))),records,negative,canonical_catalog_and_contracts_unchanged:true,limits:['CPU native artwork checks only; no browser, physical scene or gameplay claim.','Context additions do not add controls for the82remaining uncontrolled patch instances.']};
await writeFile(join(stage,'verification.json'),JSON.stringify(result,null,2)+'\n',{flag:'wx'});
console.log(JSON.stringify({status:result.status,contracts:records.length,phase_cases:records.reduce((n,r)=>n+r.phases.length,0),negative:negative.length,changed:records.map(r=>[r.id,r.phases[0].changed_pixels])}));
