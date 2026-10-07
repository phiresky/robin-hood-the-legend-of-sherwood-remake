// Private production viewport proof. No catalog or approved model is changed.
import * as THREE from 'three';
import {GLTFLoader} from 'three/examples/jsm/loaders/GLTFLoader.js';
import {EditorViewport} from '/src/editor-viewport.ts';
import {isEffectivelyVisible} from '/src/patch-display.ts';
window.apertureStage='module-start';
const prefix='/@fs/home/phire/data/dev/2026/robin-hood-the-legend-of-sherwood/';
const base=prefix+'level-editor/work/croisement02-refinement/';
const json=async path=>{const response=await fetch(path);if(!response.ok)throw Error(path);return response.json()};
const plan=await json(base+'restart10-hole-aperture/packet-v1/integration-plan.json');
const exported=await json(base+'restart10-hole-aperture/export-v1/report.json');
const routes=await json(base+'restart7-source-patch-delivery/catalog-private-v1/manifest.json');
const catalog=await json(base+'restart7-source-patch-delivery/catalog-private-v1/index.json');
window.apertureStage='source-json-loaded';
const modelBytes=new Uint8Array(await(await fetch('/@fs'+exported.model)).arrayBuffer());
const hash=async bytes=>[...new Uint8Array(await crypto.subtle.digest('SHA-256',bytes))].map(n=>n.toString(16).padStart(2,'0')).join('');
if(await hash(modelBytes)!==exported.model_sha256)throw Error('Private aperture export hash mismatch');
window.apertureStage='model-hash-verified';
const gltf=await new GLTFLoader().parseAsync(modelBytes.buffer,'');
window.apertureStage='gltf-parsed';
const scene=name=>{const result=gltf.scenes.find(s=>s.name===name);if(!result)throw Error('Missing scene '+name);return result};
const original=new THREE.Group();original.rotation.x=Math.PI/2;original.add(scene('original-receivers'));
const preparedRoot=new THREE.Group();preparedRoot.rotation.x=Math.PI/2;preparedRoot.add(scene('aperture-receivers'));
const sin=Math.sin(35*Math.PI/180),cos=Math.cos(35*Math.PI/180),bindings=[];
for(const hole of plan.bindings){
 const parent=new THREE.Group();parent.name=hole.id;
 for(const phase of ['initial','applied']){
  const endpoint=scene('endpoint-'+phase).clone(true);endpoint.name=hole.id+'/'+phase;
  endpoint.position.add(new THREE.Vector3(hole.display_position[0],hole.support_z,(hole.display_position[1]+cos*hole.support_z)/sin));
  endpoint.userData[phase==='initial'?'reveal_hide_when_applied':'reveal_show_when_applied']=hole.trigger_ids;
  parent.add(endpoint);
 }
 preparedRoot.add(parent);
 for(const id of hole.trigger_ids)bindings.push({family:id,mission:id.slice(8,id.lastIndexOf('-patch-')),patches:[id],endpointParent:parent});
}
const cache=new Map(),errors=[];
async function bytes(path){
 if(path==='private/receivers-and-endpoints.glb')return modelBytes;
 const route=routes.files[path],url=route?'/@fs'+route.source:prefix+'level-editor/library/'+path;
 if(!cache.has(url))cache.set(url,fetch(url).then(async r=>{if(!r.ok)throw Error('Missing '+path);return new Uint8Array(await r.arrayBuffer())}));
 return cache.get(url);
}
function directory(prefix=''){return{async getDirectoryHandle(name){return directory(prefix+name+'/')},async getFileHandle(name){return{async getFile(){return new File([await bytes(prefix+name)],name)}}}}}
const library=directory();let activeMission='',currentContract,level;
const viewport=new EditorViewport({document:()=>null,selection:()=>null,level:()=>level,showObstacles:()=>false,showElevation:()=>false,onSelection:()=>{},commitTransform:()=>{throw Error('Unexpected editor edit')},onError:e=>errors.push(e)});
viewport.setup(document.querySelector('#view'));viewport.replaceMap(new THREE.Group(),original,new Map());
viewport.scene.add(new THREE.HemisphereLight(0xffffff,0x777777,2));
const light=new THREE.DirectionalLight(0xffffff,2);light.position.set(-200,1000,600);viewport.scene.add(light);
async function mission(name){
 const entry=catalog.entries.find(e=>e.kind==='native-patch'&&e.mission===name&&e.name.startsWith('Croisement01 - hole'));
 if(!entry)throw Error('No hole source contract '+name);
 const sourceContract=JSON.parse(new TextDecoder().decode(await bytes(entry.contract.path)));
 const native=structuredClone(sourceContract.native);
 const selected=bindings.filter(b=>b.mission===name),ids=new Set(selected.map(b=>b.family));
 // Keep the complete pinned mission presentation; physical bindings are scoped below.
 if(selected.some(b=>!native.patch_states.some(s=>s.id===b.family)))throw Error('Missing source hole phase');
 const asset=(phase,hole)=>({id:'hole-'+phase,role:'objects',model:'private/receivers-and-endpoints.glb',model_sha256:exported.model_sha256,model_scene:'endpoint-'+phase,resources:[],position:[hole.display_position[0],hole.support_z,(hole.display_position[1]+cos*hole.support_z)/sin]});
 currentContract={version:1,scope:'controlled-state-preview',native,families:selected.map(b=>{
  const p=native.patch_states.find(s=>s.id===b.family),hole=plan.bindings.find(h=>h.trigger_ids.includes(b.family));
  return{id:b.family,element_ids:[],patch_ids:[b.family],background_ids:[],body_terminal_tick:Math.max(1,p.transition.reduce((n,f)=>n+f.delay+1,0)-1),physical:{initial:[asset('initial',hole)],applied:[asset('applied',hole)]}};})};
 const sourceData=JSON.parse(new TextDecoder().decode(await bytes(entry.mission_data.path)));
 level=JSON.parse(new TextDecoder().decode(await bytes(entry.level_data.path)));
 await viewport.setStateDelivery(currentContract,library,{name,data:sourceData,level,camera:{kind:'oblique-orthographic',elevation_deg:35}});
 viewport.setStateAperturePreview({root:preparedRoot,originalReceivers:[original],bindings});
 viewport.setDeliveredStateMode('physical-endpoint');activeMission=name;
 return snapshot();
}
function frame(holeId,opposite=false){
 const h=plan.bindings.find(h=>h.id===holeId);if(!h)throw Error(holeId);
 const center=new THREE.Vector3(h.display_position[0]+49,h.support_z,(h.display_position[1]+56+cos*h.support_z)/sin);
 const direction=new THREE.Vector3(opposite ? 0.6 : 0,sin,opposite?-cos:cos).normalize();
 viewport.orbit.target.copy(center);viewport.camera.position.copy(center).addScaledVector(direction,1600);viewport.camera.lookAt(center);viewport.camera.zoom=1;
 const aspect=960/640,half=75;viewport.camera.left=-half*aspect;viewport.camera.right=half*aspect;viewport.camera.top=half;viewport.camera.bottom=-half;viewport.camera.updateProjectionMatrix();viewport.orbit.update();viewport.renderer.render(viewport.scene,viewport.camera);
 return{center:center.toArray(),camera:viewport.camera.matrixWorld.toArray(),native:!opposite};
}
function snapshot(){
 const caps=[];preparedRoot.traverse(o=>{if(o.userData.aperture_component&&o.userData.aperture_component!=='outside')caps.push({id:o.userData.aperture_component,visible:isEffectivelyVisible(o)})});
 return{mission:activeMission,originalVisible:isEffectivelyVisible(original),preparedVisible:isEffectivelyVisible(preparedRoot),caps,endpoints:bindings.filter(b=>b.mission===activeMission).map(b=>({family:b.family,initial:isEffectivelyVisible(b.endpointParent.children[0]),applied:isEffectivelyVisible(b.endpointParent.children[1])})),errors:[...errors]};
}
function endpoint(id,phase){viewport.selectDeliveredEndpoint(id,phase);return snapshot()}
function pixels(){viewport.renderer.render(viewport.scene,viewport.camera);return viewport.renderer.domElement.toDataURL('image/png').split(',')[1]}
window.apertureStage='mission-loading';
await mission('Emb05_FoB_MP');frame('hole-14');
window.apertureProof={ready:true,viewport,plan,exported,mission,frame,snapshot,endpoint,pixels,reset(id){viewport.resetDeliveredState(id);return snapshot()},native(){viewport.setDeliveredStateMode('native-art');return snapshot()},physical(){viewport.setDeliveredStateMode('physical-endpoint');return snapshot()},clear(){viewport.clearStateDelivery();return snapshot()},dispose(){viewport.dispose();return{canvases:document.querySelectorAll('canvas').length,errors}}};
