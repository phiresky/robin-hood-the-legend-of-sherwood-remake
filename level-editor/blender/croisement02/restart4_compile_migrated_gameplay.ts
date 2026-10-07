/** Compile private placed-map gameplay before and after regrouping. */
import fs from 'node:fs';import path from 'node:path';import assert from 'node:assert/strict';import crypto from 'node:crypto';
import {parseStoredMap} from '../../shared/src/stored-level.ts';
import {compileAssetGameplay} from '../../shared/src/compile-asset-gameplay.ts';
const root=path.resolve('level-editor/work/croisement02-refinement/restart4-gameplay-migration-v4');const stage=path.resolve(process.argv[2]??'level-editor/work/croisement02-refinement/restart2-textures/post-batch15-static-candidate-v1');const folder=path.join(root,process.argv[3]??'runtime-compile-v1');assert(!fs.existsSync(folder));fs.mkdirSync(folder);
const read=(p:string)=>JSON.parse(fs.readFileSync(p,'utf8'));const hash=(s:string)=>crypto.createHash('sha256').update(s).digest('hex');
function descriptors(root:string){return new Map(read(path.join(root,'index.json')).assets.filter((e:any)=>e.source_map?.toLowerCase()==='croisement02').map((e:any)=>[e.id,read(path.join(root,e.descriptor))]));}
const old=descriptors(path.resolve('level-editor/library/3d-assets')),next=descriptors(path.join(stage,'map-assets/3d-assets'));const plan=read(path.join(root,'publication-handoff.json'));const expected=new Map(plan.descriptor_updates.map((r:any)=>[r.id,read(r.candidate_descriptor)]));
for(const[id,migrated]of expected as any){const target:any=next.get(id);assert(target);for(const key of['id','source_map','source_origin_scene','parts','model_scene'])assert.deepEqual(key==='parts'?target.parts.filter((p:any)=>p.node!=='scenery-gameplay-'+id):target[key],migrated[key],id+'/'+key);target.gameplay=migrated.gameplay;}
const terrain:any=next.get('croisement02-terrain');assert(terrain);terrain.gameplay=structuredClone((old.get('croisement02-terrain')as any).gameplay);
const retained=read(plan.retained_placement_packet).rows;const before=read('level-editor/library/scenes/croisement02.rhlos-map.json'),after=read(path.join(stage,'croisement02.rhlos-map.json'));
for(const r of retained){next.set(r.id,read(r.descriptor));if(!after.placements.some((x:any)=>x.assets.includes(r.id)))after.placements.push(r.placement);if(!after.assetSources.some((x:any)=>x.id===r.id))after.assetSources.push(r.asset_source);}
for(const id of plan.retire_ids)assert(!after.placements.some((p:any)=>p.assets.includes(id)),'Retired placement remains '+id);
const frames=[];
for(const [id,target] of next as any){
 if(!target.gameplay?.movementTransitions?.length || !expected.has(id))continue;
 const node='scenery-gameplay-'+id;
 const part={node,name:'State gameplay frame',scenery:true,gameplay_only:true};
 const existing=target.parts.find((p:any)=>p.node===node);if(existing)assert.deepEqual(existing,part);else target.parts.push(part);frames.push({id,part,glb_node:{name:node,extras:{gameplay_only:true,scenery:true}},contract:'Empty identity child of the sole logical group below the map wrapper; no mesh, skin, transform, or hidden flag. Existing mesh nodes and placement overrides unchanged.'});
}
fs.writeFileSync(path.join(folder,'required-gameplay-frames.json'),JSON.stringify(frames,null,2)+'\n');
for(const[label,raw,assets]of[['before',before,old],['after',after,next]]as const){
 console.log('COMPILING',label);const doc=parseStoredMap(raw,assets as any);const result=compileAssetGameplay(doc,assets as any,[0,0,1792,1152],{bestEffort:true});const text=JSON.stringify(result);fs.writeFileSync(path.join(folder,label+'.json'),text+'\n');const summary=Object.fromEntries(Object.entries(result).map(([k,v])=>[k,Array.isArray(v)?v.length:typeof v]));fs.writeFileSync(path.join(folder,label+'-summary.json'),JSON.stringify({result_sha256:hash(text),summary,warnings:result.warnings},null,2)+'\n');console.log(label,JSON.stringify(summary));
}
