import test from 'node:test';
import assert from 'node:assert/strict';
import * as THREE from '../../../app/node_modules/three/build/three.module.js';
import {evaluatePhysicalReceiver} from './receiver-evaluation.mjs';
const mesh=points=>{const g=new THREE.BufferGeometry();g.setAttribute('position',new THREE.Float32BufferAttribute(points.flat(),3));return new THREE.Mesh(g)};
test('evaluated top triangles retain parent placement and slope but reject nearly vertical sides',()=>{
 const child=mesh([[0,0,0],[0,2,2],[2,0,0], [0,0,0],[0,2,0],[0,2,2]]),parent=new THREE.Group();parent.position.set(10,20,30);parent.rotation.z=1e-8;parent.add(child);
 const rows=evaluatePhysicalReceiver(THREE,child,'bank-ramp',Math.PI/6);
 assert.equal(rows.length,1);assert.equal(rows[0].id,'bank-ramp:triangle-0');
 assert.ok(Math.abs(rows[0].points[0][0]-10)<1e-8);assert.ok(Math.abs(rows[0].points[0][1]-15)<1e-8);assert.ok(Math.abs(rows[0].points[0][2]-20*Math.cos(Math.PI/6))<1e-8);
 assert.ok(rows[0].points[1][2]>rows[0].points[0][2]);
});
test('unsupported dynamic meshes and missing top support fail explicitly',()=>{
 const vertical=mesh([[0,0,0],[0,2,0],[0,2,2]]);assert.throws(()=>evaluatePhysicalReceiver(THREE,vertical,'side',Math.PI/6),/no upward/);
 vertical.isSkinnedMesh=true;assert.throws(()=>evaluatePhysicalReceiver(THREE,vertical,'side',Math.PI/6),/static/);
});
