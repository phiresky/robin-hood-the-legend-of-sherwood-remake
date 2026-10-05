import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
const source=readFileSync('level-editor/refinement/browser/publication-check.js','utf8');
const cardsExpression=source.match(/const cards=\(\)=>(.*);\n/)[1];
const ids=['navigation-001','navigation-002','navigation-003'];
const cards=ids.map(id=>({dataset:{assetId:id},name:'Conditional navigation boundary'}));
for(const id of ids){
 const actual=runInNewContext(cardsExpression,{document:{querySelectorAll:()=>cards},asset:{id}});
 assert.equal(actual.length,1);assert.equal(actual[0].dataset.assetId,id);
}
assert.equal(runInNewContext(cardsExpression,{document:{querySelectorAll:()=>cards},asset:{id:'missing'}}).length,0);
const currentParts=source.match(/const currentParts=(\(\)=>\{.*?\});\n/)[1];
let rows=[];
const parts=runInNewContext(currentParts,{document:{querySelectorAll:()=>rows},index:0});
assert.equal(parts().length,0,'Transiently absent header must remain pending');
const end={classList:{contains:()=>false}};
const child2={classList:{contains:name=>name==='depth-1'},nextElementSibling:end};
const child1={classList:{contains:name=>name==='depth-1'},nextElementSibling:child2};
rows=[{nextElementSibling:child1}];
assert.equal(parts().length,2);assert.equal(parts()[0],child1);assert.equal(parts()[1],child2);
rows=[];assert.equal(parts().length,0);rows=[{nextElementSibling:child1}];assert.equal(parts().length,2);
console.log('PASS: duplicate display names select distinct IDs; absent/reappearing rows preserve exact part counts');
