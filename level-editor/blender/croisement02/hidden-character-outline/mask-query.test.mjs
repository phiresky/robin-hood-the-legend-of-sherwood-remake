import test from 'node:test';import assert from 'node:assert/strict';import {readFileSync} from 'node:fs';import {resolve} from 'node:path';
import {initialMaskMembership,queryCharacterMasks} from './mask-query.mjs';
const root=resolve(import.meta.dirname,'../../../..');
const read=name=>JSON.parse(readFileSync(resolve(root,'datadirs/fullgame_gog_hackable/Data/Levels',name)));
test('S03 load baseline disables exactly four new proto masks',()=>{
 const l=read('Croisement02.rhp.json'),m=read('S03_FoB_MP.rhm.json');
 const state=initialMaskMembership(l.masks,[...l.patches,...m.mission_patches]);
 assert.deepEqual(state.flatMap((on,i)=>on?[]:[i]),[138,139,140,141]);
});
test('cell first encounter can reverse global stream order; duplicates stay first',()=>{
 const create=(x,w)=>({box_top_left:[x,0],box_size:[w,30],layer:0,mask_type:1,character_polyline:[[0,40],[200,40]]});
 const masks=[create(70,20),create(0,150)];
 assert.deepEqual(queryCharacterMasks(masks,[true,true],{layer:0,mapPosition:[80,10]},[0,0,100,20],[4,4]),[1,0]);
 assert.deepEqual(queryCharacterMasks(masks,[true,false],{layer:0,mapPosition:[80,10]},[0,0,100,20],[4,4]),[0]);
});
test('patch references are layer-local; later load mutations win',()=>{
 const masks=[{layer:1},{layer:0},{layer:1}];
 const patches=[{old_masks:[],new_masks:[{layer:1,index:1}]},{old_masks:[{layer:1,index:1}],new_masks:[{layer:0,index:0}]}];
 assert.deepEqual(initialMaskMembership(masks,patches),[true,false,true]);
 assert.throws(()=>initialMaskMembership(masks,[{old_masks:[],new_masks:[{layer:2,index:0}]}]));
});
