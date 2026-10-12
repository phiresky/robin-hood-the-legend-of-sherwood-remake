import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import {loadProductionGateInputs} from './restart56_production_gate_browser_fixture.mjs';
const root=path.resolve(import.meta.dirname,'../../..');
const request=JSON.parse(fs.readFileSync(path.join(root,'level-editor/work/active/york/restart2/restart56-production-gate-fixture-v1/request.json')));
test('pinned HTTP inputs supply exact production sources, real model bytes and strict read-only paths',async()=>{
 const original=globalThis.fetch;
 globalThis.fetch=async url=>new Response(fs.readFileSync(url));
 try{
  const input=await loadProductionGateInputs(request,p=>p);
  assert.equal(input.source.name,'S05_Yrk_EC');assert.equal(input.contract.families[0].physical.motion.translation_frames.length,45);
  const model=input.contract.families[0].physical.initial[0];let dir=input.library;const parts=model.model.split('/');const name=parts.pop();for(const part of parts)dir=await dir.getDirectoryHandle(part);
  const file=await(await dir.getFileHandle(name)).getFile();assert.ok(file.size>1000);
  await assert.rejects(dir.getFileHandle('lossy.glb'),/Not in pinned/);
  const data=await input.read(input.contract.native.background);assert.equal(data[0],137);
  await assert.rejects(input.read({...input.contract.native.background,sha256:'0'.repeat(64)}),/Unknown native resource/);
  assert.ok(input.verifiedResources().length>=5);
 }finally{globalThis.fetch=original;}
});
test('source drift is rejected before a production loader can run',async()=>{
 const original=globalThis.fetch;globalThis.fetch=async()=>new Response('{}');
 try{await assert.rejects(loadProductionGateInputs(request,p=>p),/HTTP source hash differs/);}finally{globalThis.fetch=original;}
});
test('absent initial endpoints expose no fabricated model paths',async()=>{
 const {createHash}=await import('node:crypto');
 const contract=JSON.parse(fs.readFileSync(request.contract.path));contract.families[0].physical.initial={kind:'absent'};
 const bytes=Buffer.from(JSON.stringify(contract)),copy=structuredClone(request);copy.contract.sha256=createHash('sha256').update(bytes).digest('hex');
 const original=globalThis.fetch;globalThis.fetch=async url=>new Response(url===copy.contract.path?bytes:fs.readFileSync(url));
 try{const input=await loadProductionGateInputs(copy,p=>p);assert.deepEqual(input.contract.families[0].physical.initial,{kind:'absent'});}finally{globalThis.fetch=original;}
});
