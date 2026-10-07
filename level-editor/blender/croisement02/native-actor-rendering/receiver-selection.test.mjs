import test from 'node:test';
import assert from 'node:assert/strict';
import {projectedTerrainReceivers} from './receiver-selection.mjs';
import {projectShadowReceivers} from './receiver-shadow.mjs';
test('raised evaluated terrain follows camera rays instead of treating world Y as map Y',()=>{
 const rows=[{id:'a',points:[[0,10,10],[10,10,10],[0,20,10]]},{id:'b',points:[[10,10,10],[10,20,10],[0,20,10]]}];
 const selected=projectedTerrainReceivers(rows,[0,0,10,10]);
 assert.deepEqual(selected.map(r=>r.points),[[[0,0,10],[10,0,10],[0,10,10]],[[10,0,10],[10,10,10],[0,10,10]]]);
 const result=projectShadowReceivers({anchor:[0,0,0],bounds:{left:0,top:0,width:10,height:10},elevation:Math.PI/4,triangles:selected.map(r=>r.points)});
 assert.equal(result.coveredArea,100);assert.equal(result.ranges.length,2);
 assert.ok(result.positions.filter((_,i)=>i%3===1).every(y=>Math.abs(y-(10/Math.cos(Math.PI/4)+0.15))<1e-9));
});
test('overlapping evaluated surfaces remain explicit and are rejected instead of silently flattened',()=>{
 const rows=[{id:'a',points:[[0,0,0],[10,0,0],[0,10,0]]},{id:'b',points:[[0,4,4],[10,4,4],[0,14,4]]}];
 const selected=projectedTerrainReceivers(rows,[0,0,10,10]);assert.equal(selected.length,2);
 assert.throws(()=>projectShadowReceivers({anchor:[0,0,0],bounds:{left:0,top:0,width:10,height:10},elevation:Math.PI/4,triangles:selected.map(r=>r.points)}),/Overlapping/);
 assert.throws(()=>projectedTerrainReceivers([rows[0],rows[0]],[0,0,10,10]),/duplicate/);
});
