import test from 'node:test';
import assert from 'node:assert/strict';
import {shadowCoverageBounds} from './shadow-bounds.mjs';
test('all nonzero shadow alpha, including distant faint pixels, bounds exact source texel edges',()=>{
 const p=new Uint8ClampedArray(6*8*4),b={left:-3,top:5,width:6,height:8};p[(2*6+1)*4+3]=255;p[(7*6+4)*4+3]=1;
 assert.deepEqual(shadowCoverageBounds(p,b),{left:-2,top:3,width:4,height:6});
 assert.equal(shadowCoverageBounds(new Uint8Array(p.length),b),null);assert.throws(()=>shadowCoverageBounds(p.slice(1),b),/Invalid/);
});
