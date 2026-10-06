import fs from 'node:fs/promises';
import path from 'node:path';
import crypto from 'node:crypto';
import assert from 'node:assert/strict';
import { translateGameplayFrames } from '../../pipeline/src/translate-gameplay-frames.ts';
const root=process.cwd(), run=path.join(root,'level-editor/work/croisement03-refinement/restart2');
const sha=b=>crypto.createHash('sha256').update(b).digest('hex');
const read=async p=>JSON.parse(await fs.readFile(p,'utf8'));
function equal(a,b,where='root'){
 if(typeof a==='number'&&typeof b==='number'){assert.ok(Math.abs(a-b)<1e-9,`${where}: ${a} != ${b}`);return;}
 if(Array.isArray(a)){assert.ok(Array.isArray(b));assert.equal(a.length,b.length);a.forEach((x,i)=>equal(x,b[i],`${where}[${i}]`));return;}
 if(a&&typeof a==='object'){assert.deepEqual(Object.keys(a).sort(),Object.keys(b).sort());for(const k of Object.keys(a))equal(a[k],b[k],`${where}.${k}`);return;}
 assert.equal(a,b,where);
}
const mapPath=path.join(root,'level-editor/library/scenes/croisement03.rhlos-map.json');const mapBytes=await fs.readFile(mapPath), map=JSON.parse(mapBytes);const out=path.join(run,'approved-stone-integration-preflight/metadata-parity-v3');
await fs.mkdir(out);
const specs=[['croisement03-east-tree-rocks','approved-stone-integration-preflight/eastrocks-export-v1',[74,75]],['croisement03-central-shrub-boulder','approved-stone-integration-preflight/boulder-export-v1',[70]],['croisement03-southeast-stone-wall','wall-tree-integration-preflight/wall-export-v1',[87,88,89,90,91,92,93]]];
const reports=[];
for(const [id,relative,nodes] of specs){
 const base=path.join(run,relative),receipt=await read(path.join(base,'receipt.json'));
 for(const [name,digest]of Object.entries(receipt.files))assert.equal(sha(await fs.readFile(path.join(base,name))),digest);
 const descriptorPath=path.join(base,'3d-assets',id,'asset.json');const descriptor=await read(descriptorPath);const [ox,oy,oz]=descriptor.source_origin_scene;const target={dx:ox,dy:-oy*Math.sin(35*Math.PI/180),dz:oz*Math.cos(35*Math.PI/180),rot_deg:0};assert.deepEqual(descriptor.parts.map(p=>p.source_obstacle).sort((a,b)=>a-b),nodes);
 const fragments=[];const protectedFiles={};let maxObstacleError=0;
 for(const index of nodes){
  const native=`building-${String(index).padStart(3,'0')}`;const matches=[];
  for(const placement of map.placements)for(const asset of placement.assets){
   const ref=map.assetSources.find(s=>s.id===asset);if(!ref)throw Error(`Missing pinned source ${asset}`);
   const p=path.join(root,'level-editor/library',ref.descriptor);const raw=await read(p);
   if(raw.parts.some(part=>part.source_obstacle===index))matches.push({placement,ref,p,raw});
  }
  assert.equal(matches.length,1);const {placement,ref,p,raw}=matches[0];assert.equal(raw.parts.length,1,'Compound source requires explicit split');assert.equal(placement.assets.length,1);assert.equal(placement.transform.rot_deg,0);assert.equal(sha(await fs.readFile(p)),ref.descriptor_sha256);protectedFiles[p]=ref.descriptor_sha256;
  const old=raw.parts[0],part=descriptor.parts.find(p=>p.source_obstacle===index);assert.equal(old.node,native);assert.equal(part.node,native);assert.ok(old.default_hidden===undefined||typeof old.default_hidden==='boolean');assert.ok(part.default_hidden===undefined||typeof part.default_hidden==='boolean');assert.equal(Boolean(old.default_hidden),Boolean(part.default_hidden));const before=old.obstacle_local_game,after=part.obstacle_local_game;equal({...before,points:[]},{...after,points:[]});assert.equal(before.points.length,after.points.length);
  before.points.forEach((a,i)=>{for(const [k,t]of [['x','dx'],['y','dy'],['z_bottom','dz'],['z_top','dz']]){const error=Math.abs(a[k]+placement.transform[t]-after.points[i][k]-target[t]);assert.ok(error<1e-9);maxObstacleError=Math.max(maxObstacleError,error);}});
  const offset=[placement.transform.dx-target.dx,placement.transform.dy-target.dy,placement.transform.dz-target.dz];const translated=translateGameplayFrames(raw.gameplay,new Map([[native,offset]]));const originalWorld=translateGameplayFrames(raw.gameplay,new Map([[native,[placement.transform.dx,placement.transform.dy,placement.transform.dz]]]));const proposedWorld=translateGameplayFrames(translated,new Map([[native,[target.dx,target.dy,target.dz]]]));equal(originalWorld,proposedWorld);
  const negative=structuredClone(proposedWorld);const spatial=[...negative.surfaces,...(negative.movementBlockers??[]),...(negative.movementClearances??[])][0];let negativeProbe='no-spatial-record';if(spatial){spatial.polygon[0][0]+=1;assert.throws(()=>equal(originalWorld,negative));negativeProbe='PASS deliberately shifted gameplay point rejected';}
  fragments.push({native,prior_asset:raw.id,prior_placement:placement,source_descriptor_sha256:ref.descriptor_sha256,offset,translated_gameplay:translated,original_gameplay:raw.gameplay,world_equivalence:true,negativeProbe});
 }
 const merged={};for(const {translated_gameplay:g}of fragments)for(const [k,v]of Object.entries(g)){
  if(Array.isArray(v)){merged[k]??=[];merged[k].push(...structuredClone(v));}
  else if(k==='sightOrder'){merged[k]??={};for(const [n,o]of Object.entries(v)){assert.ok(!(n in merged[k]));merged[k][n]=o;}}
  else if(k==='draft'){merged[k]??={issues:[]};assert.deepEqual(Object.keys(v),['issues']);merged[k].issues.push(...v.issues);}
  else if(k in merged)assert.deepEqual(merged[k],v);else merged[k]=structuredClone(v);
 }
 for(const [k,v]of Object.entries(merged))if(Array.isArray(v)){const ids=v.filter(x=>x?.id).map(x=>x.id);assert.equal(new Set(ids).size,ids.length,`Duplicate ${k} IDs`);assert.equal(v.length,fragments.reduce((n,f)=>n+(f.translated_gameplay[k]?.length??0),0));}
 const proposal={...descriptor,gameplay:merged};const dest=path.join(out,id);await fs.mkdir(dest);await fs.writeFile(path.join(dest,'asset-metadata-proposal.json'),JSON.stringify(proposal,null,2)+'\n');const report={asset:id,status:'PASS existing world gameplay and obstacle metadata preserved in private proposal',proposed_transform:target,maximum_obstacle_world_error:maxObstacleError,fragments,protectedFiles,limitations:['Preserves existing recovered gameplay including its unresolved draft issues; does not certify original-game parity.','Private raw export metadata proposal only; canonical descriptor validation, browser checks and coordinated publication remain.']};await fs.writeFile(path.join(dest,'parity.json'),JSON.stringify(report,null,2)+'\n');reports.push({asset:id,parts:nodes.length,maximum_obstacle_world_error:maxObstacleError,world_gameplay_fragments_verified:fragments.length,negative_checks:fragments.map(x=>x.negativeProbe)});
 for(const [p,h]of Object.entries(protectedFiles))assert.equal(sha(await fs.readFile(p)),h);
}
assert.equal(sha(await fs.readFile(mapPath)),sha(mapBytes));await fs.writeFile(path.join(out,'receipt.json'),JSON.stringify({status:'PASS private wall, boulder and eastern-rock metadata preparation',live_map_sha256:sha(mapBytes),live_map_unchanged:true,reports},null,2)+'\n');console.log(JSON.stringify(reports,null,2));
