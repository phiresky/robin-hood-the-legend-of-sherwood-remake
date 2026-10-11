import test from 'node:test';
import assert from 'node:assert/strict';
import * as THREE from 'three';
import type { Level3D, SceneAssetSource } from '@rle/shared';
import {prepareStateAperture,verifyStateApertureReceivers,validateStateApertureManifest,type StateApertureManifest} from './state-aperture-loader.ts';
import {StateAperturePreview} from './state-aperture-preview.ts';
function fixture(){
 const source=(id:string):SceneAssetSource=>({id,role:'objects',model:id+'.glb',model_scene:id,model_sha256:'a'.repeat(64),resources:[]});
 const transform={dx:0,dy:0,dz:0,rot_deg:0};
 const manifest:StateApertureManifest={version:1,map:'test',receivers:[{asset:'bank',group:'bank',model_sha256:'a'.repeat(64),descriptor_sha256:'b'.repeat(64),placement:transform,parts:['asset:bank:part'],replacement:source('receiver')}],endpoints:{initial:source('initial'),applied:source('applied')},sites:[{id:'site',patches:['patch'],endpoint_position:[3,4,5]}],controls:[{id:'patch',mission:'mission',family:'family',site:'site',patches:['patch']}]};
 const document={map:'test',sceneAssets:[],assetSources:[{id:'bank',model_sha256:'a'.repeat(64),descriptor_sha256:'b'.repeat(64)}],groups:[{id:'bank',transform}],objects:[{id:'part',node:'asset:bank:part',group:'bank',transform}]}as unknown as Level3D;
 const original=new THREE.Group(),originals=new Map([['bank',original]]);let disposed=0;
 const load=async(source:SceneAssetSource)=>{const root=new THREE.Group();if(source.id==='receiver'){const cap=new THREE.Group();cap.userData={aperture_component:'site',reveal_hide_when_applied:['patch']};root.add(cap)}return root};
 return {manifest,document,original,originals,loader:()=>({load,dispose(){disposed++}}),disposals:()=>disposed};
}
test('reviewed receiver and real preparation connect focused cap lifecycle',async()=>{
 const f=fixture(),prepared=await prepareStateAperture(f.manifest,f.document,{}as FileSystemDirectoryHandle,f.originals,'mission','family',f.loader);
 assert.equal(f.disposals(),1);assert.equal(prepared.root.rotation.x,Math.PI/2);const controller=new StateAperturePreview(prepared);controller.selectMission('mission');controller.selectEnabled(true);assert.equal(f.original.visible,false);controller.selectEndpoint('family','applied');const site=prepared.root.getObjectByName('site')!;assert.equal(site.children[0]!.visible,false);assert.equal(site.children[1]!.visible,true);controller.dispose();assert.equal(f.original.visible,true);
});
test('source, placement and individual part edits invalidate aperture substitution',()=>{
 for(const edit of [(d:Level3D)=>d.assetSources![0]!.model_sha256='c'.repeat(64),(d:Level3D)=>d.groups[0]!.transform.dx=2,(d:Level3D)=>d.objects[0]!.hidden=true,(d:Level3D)=>d.objects.push({...d.objects[0]!,id:'copy'})]){const f=fixture();const document=structuredClone(f.document);edit(document);assert.throws(()=>verifyStateApertureReceivers(f.manifest,document),/changed/)}
});
test('unsafe resource paths and unknown focused controls are rejected',async()=>{
 const f=fixture(),unsafe=structuredClone(f.manifest);unsafe.endpoints.applied.model='../outside.glb';assert.throws(()=>validateStateApertureManifest(unsafe),/Invalid/);
 await assert.rejects(prepareStateAperture(f.manifest,f.document,{}as FileSystemDirectoryHandle,f.originals,'other','family',f.loader),/focused/);
});
test('missing cap metadata rejects preparation and releases loader',async()=>{
 const f=fixture();await assert.rejects(prepareStateAperture(f.manifest,f.document,{}as FileSystemDirectoryHandle,f.originals,'mission','family',()=>({load:async()=>new THREE.Group(),dispose(){f.loader().dispose()}})),/cap binding/);assert.equal(f.disposals(),1);assert.equal(f.original.visible,true);
});
