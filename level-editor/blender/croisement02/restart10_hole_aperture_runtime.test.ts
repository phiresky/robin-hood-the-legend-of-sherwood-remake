/** Private physical-aperture proof using the production reveal visibility engine. */
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import crypto from 'node:crypto';
import * as THREE from '../../app/node_modules/three/build/three.module.js';
import {PatchDisplay,isEffectivelyVisible} from '../../app/src/patch-display.ts';
const folder=path.resolve(process.argv[2]??'level-editor/work/croisement02-refinement/restart10-hole-aperture/packet-v1');
const packet=JSON.parse(fs.readFileSync(path.join(folder,'packet.json'),'utf8'));
const root=new THREE.Group(),display=new PatchDisplay();
const material=new THREE.MeshBasicMaterial({side:THREE.DoubleSide});
const sin=Math.sin(35*Math.PI/180),cos=Math.cos(35*Math.PI/180);
function mesh(name:string,triangles:number[][][]){
 const geometry=new THREE.BufferGeometry();geometry.setAttribute('position',new THREE.Float32BufferAttribute(triangles.flat(2),3));
 const object=new THREE.Mesh(geometry,material);object.name=name;return object;
}
let exact=0,interpolated=0,maxPosition=0,maxUV=0;
for(const source of packet.meshes){
 for(const [tag,rows]of Object.entries(source.groups) as any){
  const object=mesh(source.name+'/'+tag,rows.map((r:any)=>r.xyz));
  if(tag!=='outside')object.userData.reveal_hide_when_applied=packet.holes.find((h:any)=>h.id===tag).triggers;
  root.add(object);
  for(const row of rows){
   const original=source.source[row.source_triangle];assert.equal(row.material,original.material);
   if(!row.weights){assert.deepEqual(row,original);exact++;continue;}
   interpolated++;
   for(let i=0;i<3;i++){
    assert(Math.abs(row.weights[i].reduce((a:number,b:number)=>a+b,0)-1)<1e-8);
    for(let j=0;j<3;j++)maxPosition=Math.max(maxPosition,Math.abs(row.xyz[i][j]-row.weights[i].reduce((sum:number,w:number,k:number)=>sum+w*original.xyz[k][j],0)));
    for(const [name,values]of Object.entries(original.uv)as any)for(let j=0;j<2;j++)maxUV=Math.max(maxUV,Math.abs(row.uv[name][i][j]-row.weights[i].reduce((sum:number,w:number,k:number)=>sum+w*values[k][j],0)));
   }
  }
 }
}
assert(maxPosition<1e-8&&maxUV<1e-10);
const base=root.clone(true);
for(const hole of packet.holes)for(const phase of ['initial','applied']){
 const object=mesh(hole.id+'/'+phase,packet.endpoints[phase].triangles);
 object.position.set(hole.position[0],-(hole.position[1]+cos*hole.support_z)/sin,hole.support_z);
 object.userData[phase==='initial'?'reveal_hide_when_applied':'reveal_show_when_applied']=hole.triggers;
 root.add(object);
}
function hit(scene:THREE.Group,hole:any){
 scene.updateMatrixWorld(true);
 const center=new THREE.Vector3(hole.position[0]+49,-(hole.position[1]+56)/sin,0),direction=new THREE.Vector3(0,-cos,sin);
 const ray=new THREE.Raycaster(center.clone().addScaledVector(direction,6000),direction.clone().negate());
 return ray.intersectObjects(scene.children,true).find(h=>isEffectivelyVisible(h.object));
}
function visible(){display.apply(root);return root.children.filter(isEffectivelyVisible).map(x=>x.name).sort();}
const initial=visible(),checks=[];
for(const hole of packet.holes){
 display.clear();display.apply(base);const closed=hit(base,hole);assert(closed, hole.id+' initial receiver missing');
 assert(Math.abs(closed.point.z-hole.support_z)<.003);
 display.set(hole.triggers[0],true);visible();display.apply(base);
 const exposed=hit(base,hole),bowl=hit(root,hole);
 assert(bowl&&bowl.object.name===hole.id+'/applied',hole.id+' bowl is occluded');
 assert(bowl.point.z<hole.support_z-.05,hole.id+' bowl not below support');
 assert(!exposed||exposed.point.z<bowl.point.z-.05,hole.id+' receiver remains over bowl');
 for(const other of packet.holes){const cap=base.children.find(x=>x.name.endsWith('/'+other.id));assert(cap);assert.equal(cap.visible,other.id!==hole.id);}
 display.clear();assert.deepEqual(visible(),initial);display.apply(base);const reset=hit(base,hole);assert(reset);assert(reset.point.distanceTo(closed.point)<1e-9);
 checks.push({hole:hole.id,support_z:hole.support_z,initial_z:closed.point.z,applied_z:bowl.point.z,reset_exact:true});
}
const triggers=packet.holes.map((h:any)=>h.triggers[0]);
for(const id of triggers)display.set(id,true);const forward=visible();
display.clear();for(const id of [...triggers].reverse())display.set(id,true);assert.deepEqual(visible(),forward);
for(const hole of packet.holes){display.set(hole.triggers[0],false);visible();assert(root.children.find(x=>x.name===hole.id+'/initial')!.visible);assert(!root.children.find(x=>x.name===hole.id+'/applied')!.visible);}
assert.deepEqual(visible(),initial);
const duplicate=packet.holes.find((h:any)=>h.triggers.length>1);display.set(duplicate.triggers[0],true);display.set(duplicate.triggers[1],true);display.set(duplicate.triggers[0],false);visible();assert(root.children.find(x=>x.name===duplicate.id+'/applied')!.visible);display.clear();assert.deepEqual(visible(),initial);
const report={pass:true,packet_sha256:crypto.createHash('sha256').update(fs.readFileSync(path.join(folder,'packet.json'))).digest('hex'),production_visibility_engine:'PatchDisplay',exact_original_triangles:exact,interpolated_triangles:interpolated,max_position_interpolation_error:maxPosition,max_uv_interpolation_error:maxUV,checks,all17_order_independent:true,reset_exact:true,replica_trigger_union:true,scope:'Private representation and CPU ray proof; no runtime controller hookup, live terrain replacement, appearance approval or full animated hole claim.'};
fs.writeFileSync(path.join(folder,'runtime-proof.json'),JSON.stringify(report,null,2)+'\n');console.log(JSON.stringify(report));
