import {test} from 'node:test';
import assert from 'node:assert/strict';
import {stateProofCommand} from './restart6_verify_remaining_state_editor.mjs';
class Socket {
 readyState=1; listeners=new Map();sent=[];
 addEventListener(type,fn){if(!this.listeners.has(type))this.listeners.set(type,new Set());this.listeners.get(type).add(fn)}
 removeEventListener(type,fn){this.listeners.get(type)?.delete(fn)}
 send(text){this.sent.push(JSON.parse(text))}
 emit(type,payload){for(const fn of [...this.listeners.get(type)??[]])fn({data:JSON.stringify(payload)})}
 get remaining(){return [...this.listeners.values()].reduce((n,s)=>n+s.size,0)}
}
test('response resolves only matching request and removes listeners',async()=>{const ws=new Socket();const p=stateProofCommand(ws,()=>7,'Runtime.evaluate',{},1000);ws.emit('message',{id:8,result:'unrelated'});ws.emit('message',{id:7,result:{value:42}});assert.deepEqual(await p,{value:42});assert.equal(ws.remaining,0)});
for(const event of ['Runtime.executionContextsCleared','Inspector.detached'])test(event+' rejects pending request immediately',async()=>{const ws=new Socket(),p=stateProofCommand(ws,()=>1,'Runtime.evaluate',{},1000);ws.emit('message',{method:event});await assert.rejects(p,/context replaced/);assert.equal(ws.remaining,0)});
test('closed connection rejects without hanging',async()=>{const ws=new Socket(),p=stateProofCommand(ws,()=>1,'Page.captureScreenshot',{},1000);ws.emit('close');await assert.rejects(p,/connection closed/);assert.equal(ws.remaining,0)});
test('unanswered command times out and removes listeners',async()=>{const ws=new Socket(),p=stateProofCommand(ws,()=>1,'Runtime.evaluate',{},10);await assert.rejects(p,/timed out/);assert.equal(ws.remaining,0)});
