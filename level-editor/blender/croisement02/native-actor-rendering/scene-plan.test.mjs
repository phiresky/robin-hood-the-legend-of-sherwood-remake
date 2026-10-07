import test from 'node:test';
import assert from 'node:assert/strict';
import { planCurrentScene } from './scene-plan.mjs';
const element = (identity,creationOrder,displayOrder,more={})=>({identity,creationOrder,
  displayOrder,active:true,stage:'ordered',masking:'off',epoch:3,...more});
const plan = elements=>planCurrentScene({epoch:3,tick:8,elements});
test('actors and effects share current Y order, construction ties and a separate background pass',()=>{
  const rows=[element('actor',40,200,{masking:'character',layer:0,mapPosition:[10,200],drawHidden:false}),
    element('sign',10,200),element('butterfly05',12,900,{stage:'background'}),
    element('second-restoration',11,1000,{stage:'background'}),element('hidden',9,-100,{active:false})];
  const before=structuredClone(rows), result=plan(rows);
  assert.deepEqual(result.ordered.map(x=>x.identity),['sign','actor']);
  assert.deepEqual(result.background.map(x=>x.identity),['second-restoration','butterfly05']);
  assert.deepEqual(result.hidden.map(x=>x.identity),['hidden']);assert.deepEqual(rows,before);
  rows[0].mapPosition[1]=190;rows[0].displayOrder=190;
  assert.deepEqual(plan(rows).ordered.map(x=>x.identity),['actor','sign']);
  assert.equal(result.ordered[1].mapPosition[1],200);
});
test('float32 display ties use construction rank, not incoming batch or family order',()=>{
  assert.deepEqual(plan([element('later',5,200),element('earlier',2,200+1e-7)])
    .ordered.map(x=>x.identity),['earlier','later']);
});
test('stale identities and guessed duplicate construction ranks fail before drawing',()=>{
  assert.throws(()=>plan([element('a',1,0),element('b',1,1)]),/construction rank/);
  assert.throws(()=>plan([element('a',1,0),element('a',2,1)]),/identity/);
  assert.throws(()=>plan([element('a',1,0,{epoch:2})]),/Retired/);
  assert.throws(()=>plan([element('a',1,0,{masking:'character'})]),/mask query/);
  assert.throws(()=>plan([element('a',1,0,{masking:'character',layer:0,mapPosition:[0,0],drawHidden:true})]),/outline/);
});
