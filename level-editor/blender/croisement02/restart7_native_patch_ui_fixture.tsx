// Private production controls and viewport fixture; no library publication.
import { render } from '@solidjs/web';
import { createSignal } from 'solid-js';
import * as THREE from 'three';
import StatePreview from '/src/StatePreview.tsx';
import { EditorViewport } from '/src/editor-viewport.ts';
import '/src/styles.css';
const root='/@fs/home/phire/data/dev/2026/robin-hood-the-legend-of-sherwood/',base=root+'level-editor/work/croisement02-refinement/restart7-source-patch-delivery/';
const manifest=await(await fetch(base+'catalog-private-v1/manifest.json')).json(),catalog=await(await fetch(base+'catalog-private-v1/index.json')).json();
const entries=catalog.entries.filter(e=>e.kind==='native-patch'),sourceReview=await(await fetch(base+'contracts-v1/source-review-v1/manifest.json')).json();
const errors=[],cache=new Map();let broken='',holdResolve;const held=new Promise(r=>holdResolve=r);
async function file(path){const route=manifest.files[path],url=route?'/@fs'+route.source:root+'level-editor/library/'+path;if(!cache.has(url))cache.set(url,fetch(url).then(async r=>{if(!r.ok)throw Error('Missing '+path);return new Uint8Array(await r.arrayBuffer())}));return cache.get(url)}
function directory(prefix='',fault=''){return{async getDirectoryHandle(name){return directory(prefix+name+'/',fault)},async getFileHandle(name){return{async getFile(){const path=prefix+name;if(path==='mission-states/index.json')return new File([JSON.stringify({...catalog,entries})],name);if(fault==='held'&&path.includes('/contracts/'))await held;if(fault==='missing'&&path.includes('/contracts/'))throw new DOMException('Missing '+path,'NotFoundError');const bytes=await file(path);return new File([fault==='hash'&&path.includes('/contracts/')?new Uint8Array([123,125]):bytes],name)}}}}}
const initial=entries[0],level=JSON.parse(new TextDecoder().decode(await file(initial.level_data.path)));
const [library,setLibrary]=createSignal(directory()),[active,setActive]=createSignal(true),[doc,setDoc]=createSignal({map:'Croisement02',camera:{kind:'oblique-orthographic',elevation_deg:35},mission:{version:1,importedFrom:initial.mission,spawnPoints:[],soldiers:[]}});
const viewport=new EditorViewport({document:()=>null,selection:()=>null,level:()=>level,showObstacles:()=>false,showElevation:()=>false,onSelection:()=>{},commitTransform:()=>{throw Error('Unexpected edit')},onError:e=>errors.push(e)});viewport.setup(document.querySelector('#view'));viewport.replaceMap(new THREE.Group(),null,new Map());
const unmount=render(()=> <StatePreview document={doc} library={library} viewport={viewport} active={active()} onError={e=>errors.push(e)}/>,document.querySelector('#panel'));
const wait=ms=>new Promise(r=>setTimeout(r,ms));
function select(value){const el=document.querySelector('[aria-label="State preview asset"]');el.value=value;el.dispatchEvent(new Event('change',{bubbles:true}))}
function click(name){const el=[...document.querySelectorAll('#panel button')].find(e=>e.textContent===name);if(!el)throw Error('Missing '+name);el.click()}
function frame(tick){const el=document.querySelector('[aria-label="State preview frame"]');el.value=tick;el.dispatchEvent(new Event('input',{bubbles:true}))}
function snapshot(){return{...viewport.nativeArtStatus(),mode:viewport.statePresentationMode,focus:viewport.nativePatchFocus,status:[...document.querySelectorAll('#panel [role="status"]')].map(e=>e.textContent),physicalOptions:[...document.querySelectorAll('[aria-label="State preview view"] option')].map(e=>e.value),errors:[...errors]}}
async function hash(){const bytes=viewport.currentNativeArt.pixels().data;return [...new Uint8Array(await crypto.subtle.digest('SHA-256',bytes))].map(n=>n.toString(16).padStart(2,'0')).join('')}
function crop(id){const bounds=sourceReview.images.find(r=>r.id===id).crop,pixels=viewport.currentNativeArt.pixels(),canvas=document.createElement('canvas');canvas.width=bounds[2]-bounds[0];canvas.height=bounds[3]-bounds[1];canvas.getContext('2d').putImageData(new ImageData(new Uint8ClampedArray(pixels.data),pixels.width,pixels.height),-bounds[0],-bounds[1]);return canvas.toDataURL('image/png').split(',')[1]}
window.patchProof={ready:true,crop,entries,review:sourceReview,viewport,snapshot,hash,select,click,frame,mission(name){setDoc({...doc(),mission:{...doc().mission,importedFrom:name}})},fault(name){setLibrary(directory('',name))},releaseHeld(){holdResolve()},hide(){setActive(false)},show(){setActive(true)},async dispose(){unmount();await wait(20);const retired=!viewport.nativeArtStatus().ready;viewport.dispose();return{retired,canvases:document.querySelectorAll('canvas').length}}};
