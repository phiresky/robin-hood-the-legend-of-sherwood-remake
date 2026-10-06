/** Private lossless assembly of two independently controlled hall covers. */
import fs from 'node:fs/promises';
import path from 'node:path';
import assert from 'node:assert/strict';
import { Document, NodeIO, PropertyType } from '../../pipeline/node_modules/@gltf-transform/core/dist/index.js';
import { ALL_EXTENSIONS } from '../../pipeline/node_modules/@gltf-transform/extensions/dist/index.js';
import { mergeDocuments, dedup, prune, unpartition } from '../../pipeline/node_modules/@gltf-transform/functions/dist/index.js';
import * as THREE from '../../app/node_modules/three/build/three.module.js';
import { PatchDisplay, isEffectivelyVisible } from '../../app/src/patch-display.ts';
import { modelContentSignatures, canonical, sha256 } from '../../pipeline/src/bundle-asset-states.ts';

const base = path.resolve('level-editor/work/york-refinement/restart2/hall-textures-v2/exports-v3');
const out = path.join(base, 'independent-wiring-v2');
const states = ['initial-initial', 'initial-applied', 'applied-initial', 'applied-applied'];
const io = new NodeIO().registerExtensions(ALL_EXTENSIONS);
const target = new Document();
const scene = target.createScene('default');
target.getRoot().setDefaultScene(scene);
const map = target.createNode('map'); scene.addChild(map);
const asset = target.createNode('Castle great hall').setExtras({asset_group:'york-castle-great-hall'});map.addChild(asset);
const wrappers = new Map(), sourceProofs = [], bindings = [];
let descriptor;
for (const state of states) {
  const folder = path.join(base, state, '3d-assets/york-castle-great-hall');
  const bytes = await fs.readFile(path.join(folder, 'model.glb'));
  const source = await io.readBinary(bytes);
  const report = JSON.parse(await fs.readFile(path.join(base,state,'export-report.json'),'utf8'));
  assert.equal(sha256(bytes), report.model_sha256);
  const desc = JSON.parse(await fs.readFile(path.join(folder,'asset.json'),'utf8'));
  if (!descriptor) descriptor = structuredClone(desc);
  assert.deepEqual(desc.gameplay, descriptor.gameplay);
  assert.deepEqual(desc.source_origin_scene, descriptor.source_origin_scene);
  const sourceMap = source.getRoot().listNodes().find(n=>n.getName()==='map');
  if (!bindings.length) map.setTranslation(sourceMap.getTranslation()).setRotation(sourceMap.getRotation()).setScale(sourceMap.getScale());
  assert.deepEqual(map.getMatrix(), sourceMap.getMatrix());
  const signatures = await modelContentSignatures(source);
  const merged = mergeDocuments(target, source);
  const [first, second] = state.split('-');
  const rule = {reveal_material_patch:'appearance-1',reveal_material_state:first==='initial'?'covered':'revealed',
    [second==='initial'?'reveal_hide_when_applied':'reveal_show_when_applied']:['appearance-2']};
  bindings.push({state,patch001:first,patch002:second,rule,source_sha256:sha256(bytes)});
  for (const part of desc.parts) {
    const nodes = source.getRoot().listNodes().filter(n=>n.getName()===part.node);
    assert.equal(nodes.length,1);
    const original = nodes[0], copy = merged.get(original);
    // Source exports have an identity asset frame below their common map rotation.
    assert.deepEqual(original.getParentNode().getMatrix(),new THREE.Matrix4().toArray());
    if (!wrappers.has(part.node)) {
      const wrapper = target.createNode(part.node).setExtras(original.getExtras());
      wrappers.set(part.node,wrapper);asset.addChild(wrapper);
      if (!descriptor.parts.some(p=>p.node===part.node)) descriptor.parts.push(structuredClone(part));
    }
    copy.setName(`${part.node}--${state}`).setExtras({...original.getExtras(),...rule});
    wrappers.get(part.node).addChild(copy);
    sourceProofs.push({state,node:part.node,expected:signatures.nodeData(original)});
  }
  for (const sourceScene of source.getRoot().listScenes()) merged.get(sourceScene).dispose();
}
await target.transform(prune({keepExtras:true,keepAttributes:true,keepIndices:true,keepSolidTextures:true}),dedup({propertyTypes:[PropertyType.ACCESSOR,PropertyType.TEXTURE,PropertyType.MATERIAL,PropertyType.MESH]}),unpartition());
const bytes = await io.writeBinary(target), roundtrip = await io.readBinary(bytes);
const signatures = await modelContentSignatures(roundtrip);
const removeRule = value => {
  if (Array.isArray(value)) return value.map(removeRule);
  if (!value || typeof value !== 'object') return value;
  return Object.fromEntries(Object.entries(value).filter(([key])=>!['reveal_material_patch','reveal_material_state','reveal_hide_when_applied','reveal_show_when_applied'].includes(key)).map(([key,v])=>[key,removeRule(v)]));
};
for (const proof of sourceProofs) {
  const node = roundtrip.getRoot().listNodes().find(n=>n.getName()===`${proof.node}--${proof.state}`);
  assert(node);
  assert.equal(canonical(removeRule(signatures.nodeData(node))),canonical(removeRule(proof.expected)),`Surface mismatch ${proof.state}/${proof.node}`);
}
// Use the production visibility evaluator for every transition in both directions.
const display = new PatchDisplay(), root = new THREE.Group();
for (const row of bindings) { const node = new THREE.Group();node.name=row.state;node.userData=row.rule;root.add(node); }
const trace=[];
for (const state of [...states, ...states.toReversed()]) {
  const [a,b]=state.split('-');display.set('appearance-1',a==='applied');display.set('appearance-2',b==='applied');display.apply(root);
  const visible=root.children.filter(isEffectivelyVisible).map(n=>n.name);assert.deepEqual(visible,[state]);trace.push({state,visible});
}
await fs.mkdir(out,{recursive:false});
descriptor.model='model.glb';delete descriptor.model_scene;delete descriptor.state_variants;delete descriptor.standalone_variants;
const assets = path.join(out,'3d-assets/york-castle-great-hall');
await fs.mkdir(assets,{recursive:true});
await fs.writeFile(path.join(assets,'model.glb'),bytes);
const descriptorBytes=JSON.stringify(descriptor,null,2)+'\n';
await fs.writeFile(path.join(assets,'asset.json'),descriptorBytes);
await fs.writeFile(path.join(out,'3d-assets/index.json'),JSON.stringify({version:1,assets:[{id:descriptor.id,name:descriptor.name,source_map:'york',descriptor:descriptor.id+'/asset.json',model:descriptor.id+'/model.glb',descriptor_sha256:sha256(descriptorBytes),model_sha256:sha256(bytes),editor:descriptor}]},null,2)+'\n');
await fs.writeFile(path.join(out,'wiring-proof.json'),JSON.stringify({status:'PASS lossless per-part geometry, transforms, UVs, encoded textures and production four-state visibility',model_sha256:sha256(bytes),source_parts_verified:sourceProofs.length,bindings,trace,appearance_patches:{'appearance-1':'patch-001','appearance-2':'patch-002'},scope:'Private appearance assembly; does not claim mission trigger reachability, collision transitions or candle animation. Exact full endpoint models remain independent reference authority.',browser_verified:false,live_writes:false},null,2)+'\n');
console.log(JSON.stringify({out,bytes:bytes.length,parts:wrappers.size,proofs:sourceProofs.length,sha256:sha256(bytes)}));
