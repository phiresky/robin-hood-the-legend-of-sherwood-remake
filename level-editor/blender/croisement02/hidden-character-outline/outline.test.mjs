import test from 'node:test';import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';import {resolve} from 'node:path';import {createHash} from 'node:crypto';
import {decodeLegacy,outlineCharacter,splitPacked,packColor} from './outline.mjs';
const root=resolve(import.meta.dirname,'../../../..');
const fixture=JSON.parse(readFileSync(resolve(root,'level-editor/work/croisement02-refinement/restart20-hidden-outline-v1/fixture.json')));
const levelBytes=readFileSync(resolve(root,fixture.level.path));const level=JSON.parse(levelBytes);
const hash=b=>createHash('sha256').update(b).digest('hex');
const mask=(bits,x=0)=>({box_top_left:[x,0],box_size:[bits.length,1],mask_type:1,layer:0,character_polyline:[[0,10],[99,10]],mask_data:[2,0x81,parseInt(bits.padEnd(8,'0'),2)]});
const surface=(data,depth=16)=>({width:data.length,height:1,data:new Uint16Array(data),depth,transparent:depth===16?0x7c0:0x3e0,shadowKey:31});
const actor={layer:0,mapPosition:[1,1]};
const run=(data,masks,opts={})=>outlineCharacter(surface(data),[0,0],actor,masks.map((m,i)=>({id:i,mask:m,active:true})),{outlineColor:0xf800,...opts});
test('real actor art and mask match independently computed scalar packed result',()=>{
 assert.equal(hash(levelBytes),fixture.level.sha256);assert.equal(hash(readFileSync(resolve(root,fixture.image.path))),fixture.image.sha256);
 const src=decodeLegacy(fixture.source,{shadowKey:31});
 const got=outlineCharacter(src,fixture.screenOrigin,fixture.actor,[{id:fixture.maskIndex,mask:level.masks[fixture.maskIndex],active:true}],{outlineColor:0xf800});
 const bytes=Buffer.alloc(got.pixels.data.length*2);got.pixels.data.forEach((v,i)=>bytes.writeUInt16LE(v,i*2));
 assert.equal(hash(bytes),fixture.expectedPackedSha256);assert.equal(got.applied[0].outlined,fixture.expectedOutlined);
 assert.notDeepEqual(got.pixels.data,src.data);
});
test('horizontal right-neighbor transitions, not dilation or color edges',()=>{
 const T=0x7c0;assert.deepEqual([...run([T,1,2,T,T],[mask('11111')]).pixels.data],[0xf800,T,0xf800,T,T]);
 // Final clipped column clears even when a body/empty transition would exist.
 assert.deepEqual([...run([1,T],[mask('11')]).pixels.data],[0xf800,T]);
 assert.deepEqual([...run([T,1],[mask('11')]).pixels.data],[0xf800,T]);
});
test('shadow key is empty for edge detection and masked shadows disappear',()=>{
 const T=0x7c0;assert.deepEqual([...run([31,1,31,T],[mask('1111')]).pixels.data],[0xf800,0xf800,T,T]);
 const split=splitPacked(surface([31,T,0xf800]));assert.deepEqual([...split.shadow],[255,0,0]);assert.deepEqual([...split.body.slice(8)],[255,0,0,255]);
});
test('mask order is preserved; second mask reads first mask output',()=>{
 const T=0x7c0,one=mask('1100'),two=mask('0110');
 const a=run([T,1,1,T],[one,two]),b=run([T,1,1,T],[two,one]);
 assert.notDeepEqual(a.pixels.data,b.pixels.data);
});
test('activity/layer gate and ordinary cutout path',()=>{
 const src=surface([1,2,3,4]),m=mask('1111');
 const out=outlineCharacter(src,[0,0],actor,[{id:0,mask:m,active:false}],{outlineColor:0xf800});assert.deepEqual(out.pixels.data,src.data);
 assert.deepEqual([...run([1,2,3,4],[m],{drawHidden:false}).pixels.data],[0x7c0,0x7c0,0x7c0,0x7c0]);
 assert.deepEqual([...src.data],[1,2,3,4]);
});
test('15-bit keys and legacy RGBA conversion reject filtered pixels',()=>{
 const src=decodeLegacy({width:3,height:1,data:[0,248,0,255,0,0,255,255,255,0,0,255]},{depth:15,shadowKey:31});
 assert.deepEqual([...src.data],[0x3e0,31,0x7c00]);assert.equal(packColor(255,0,0,15),0x7c00);
 assert.throws(()=>decodeLegacy({width:1,height:1,data:[1,2,3,128]},{shadowKey:31}));
 assert.throws(()=>decodeLegacy({width:1,height:1,data:[1,2,3,255]},{shadowKey:0x7c0}));
});
