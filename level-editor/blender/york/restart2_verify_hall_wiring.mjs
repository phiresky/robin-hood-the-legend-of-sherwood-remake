/** Check actual exported hierarchy through production placement and visibility rules. */
import fs from 'node:fs/promises';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import * as THREE from '../../app/node_modules/three/build/three.module.js';
import {PatchDisplay,applyPlacementPatches,isEffectivelyVisible} from '../../app/src/patch-display.ts';
const base='level-editor/work/york-refinement/restart2/hall-textures-v2/exports-v3/independent-wiring-v2';
const bytes=await fs.readFile(base+'/3d-assets/york-castle-great-hall/model.glb');
const json=JSON.parse(bytes.subarray(20,20+bytes.readUInt32LE(12)).toString());
const descriptor=JSON.parse(await fs.readFile(base+'/3d-assets/york-castle-great-hall/asset.json','utf8'));
const asset='york-castle-great-hall';
const make=i=>{
  const source=json.nodes[i],node=new THREE.Object3D();node.name=source.name??'';
  node.userData=structuredClone(source.extras??{});
  if(source.mesh!==undefined)node.userData.audit_mesh=true;
  for(const child of source.children??[])node.add(make(child));
  return node;
};
const source=new THREE.Object3D();for(const id of json.scenes[json.scene??0].nodes)source.add(make(id));
const originalMetadata=[];source.traverse(n=>originalMetadata.push([n.name,structuredClone(n.userData)]));
const document={groups:['a','b'].map(id=>({id,patches:{[asset]:{'appearance-1':id+'-001','appearance-2':id+'-002'}}}))};
const available=new Set(descriptor.parts.map(p=>'asset:'+asset+':'+p.node));
const root=new THREE.Object3D(),placements=new Map();
for(const group of document.groups){
  const parent=new THREE.Object3D();root.add(parent);placements.set(group.id,parent);
  for(const part of descriptor.parts){
    const node=source.getObjectByName(part.node).clone(true);
    applyPlacementPatches(node,document,{node:'asset:'+asset+':'+part.node,group:group.id},available);
    parent.add(node);
  }
}
const display=new PatchDisplay(),trace=[];
const check=(a,b)=>{
  for(const [id,state] of [['a',a],['b',b]]){
    const [first,second]=state.split('-');display.set(id+'-001',first==='applied');display.set(id+'-002',second==='applied');
  }
  display.apply(root);
  const row={a,b,visible:{}};
  for(const [id,state]of[['a',a],['b',b]]){
    const visible=[];placements.get(id).traverse(n=>{if(n.userData.audit_mesh&&isEffectivelyVisible(n)){
      assert(n.parent.name.endsWith('--'+state),'Wrong endpoint visible '+n.parent.name);
      visible.push(n.parent.name);
    }});
    assert.equal(visible.length,state.endsWith('-initial')?15:14);
    assert.equal(new Set(visible).size,visible.length);
    row.visible[id]=visible;
  }
  trace.push(row);
};
check('initial-initial','initial-initial');check('applied-initial','initial-initial');
check('applied-applied','initial-initial');check('applied-applied','initial-applied');
check('initial-applied','applied-initial');check('initial-initial','initial-initial');
const after=[];source.traverse(n=>after.push([n.name,n.userData]));assert.deepEqual(after,originalMetadata);
const result={status:'PASS actual exported hierarchy, independent placement binding and reset',model_sha256:createHash('sha256').update(bytes).digest('hex'),trace,source_metadata_unchanged:true,browser_verified:false};
await fs.writeFile(base+'/placement-wiring-proof.json',JSON.stringify(result,null,2)+'\n');
console.log(JSON.stringify({status:'PASS',traces:trace.length,placements:2,partsPerPlacement:descriptor.parts.length}));
