/** Private metadata-only migration from frozen source-part frames. */
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import crypto from 'node:crypto';
import { translateGameplayFrames } from '../../pipeline/src/translate-gameplay-frames.ts';
import { validateAssetGameplay } from '../../shared/src/asset-gameplay.ts';
const root=path.resolve('level-editor/work/croisement02-refinement/restart4-gameplay-migration-v4');
const read=(p:string)=>JSON.parse(fs.readFileSync(p,'utf8'));
const digest=(p:string)=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const inventory=read(path.join(root,'frame-inventory.json'));
assert.equal(inventory.errors.length,0,'Input frame/obstacle audit must pass');
const out=path.join(root,'migrated-descriptors');assert(!fs.existsSync(out));fs.mkdirSync(out);
const arrays=['surfaces','projectionReceivers','materials','movementBlockers','movementClearances','doors','lifts','interiors','jumpZones','jumpPairs','movementTransitions','sounds'];
const merged=new Map<string,any>(), provenance:any[]=[];let roundtripMax=0,worldMax=0,worldSamples=0;
function compare(a:any,b:any,p='') {
 if(typeof a==='number'&&typeof b==='number'){roundtripMax=Math.max(roundtripMax,Math.abs(a-b));assert(Math.abs(a-b)<1e-9,p);return;}
 if(a&&typeof a==='object'){assert.deepEqual(Object.keys(a).sort(),Object.keys(b).sort(),p);for(const k of Object.keys(a))compare(a[k],b[k],p+'/'+k);return;}assert.deepEqual(a,b,p);
}
function pointSamples(g:any):{node:string,p:number[]}[]{
 const result:{node:string,p:number[]}[]=[];
 function walk(v:any,node?:string){
  if(!v||typeof v!=='object')return;node=v.node??node;
  if(node&&Array.isArray(v.polygon)&&v.polygon.length){for(let i=0;i<v.polygon.length;i++){const p=v.polygon[i];result.push({node,p:p.length===3?p:[...p,Array.isArray(v.height)?v.height[i]:v.height??0]});}}
  for(const[k,x]of Object.entries(v)){
   if(k==='polygon')continue;
   if(node&&Array.isArray(x)&&x.length===3&&x.every(y=>typeof y==='number')&&['anchor','waypoint','outside','inside','middle','a','b','point','waypointAnchor'].includes(k)){result.push({node,p:x as number[]});continue;}
   if(x&&typeof x==='object')walk(x,node);
  }
 }
 walk(g);return result;
}
const angle=inventory.camera.elevation_deg*Math.PI/180;
function world(m:number[][],p:number[]){const v=[p[0],-p[1]/Math.sin(angle),p[2]/Math.cos(angle),1];return m.slice(0,3).map(row=>row.reduce((s,x,i)=>s+x*v[i]!,0));}
let negativeDrift=0;
for(const record of inventory.records){
 if(!record.gameplay)continue;assert.equal(digest(record.source_descriptor),inventory.pins[record.source_descriptor]);
 for(const k of Object.keys(record.gameplay))assert(['version','collision','sightOrder','draft',...arrays].includes(k),'Unimplemented gameplay field '+k);
 const offsets=new Map(Object.entries(record.offsets));const translated=translateGameplayFrames(record.gameplay,offsets as any);
 const inverse=new Map(Object.entries(record.offsets).map(([k,v]:any)=>[k,v.map((x:number)=>-x)]));compare(record.gameplay,translateGameplayFrames(translated,inverse as any));
 const before=pointSamples(record.gameplay),after=pointSamples(translated);assert.equal(before.length,after.length);
 for(let i=0;i<before.length;i++){
  const a=before[i]!,b=after[i]!;assert.equal(a.node,b.node);const frame=record.frame_checks.find((r:any)=>r.node===a.node);assert(frame,'Missing node frame '+a.node);
  const wa=world(frame.old_world,a.p),wb=world(frame.new_world,b.p);const drift=Math.max(...wa.map((x,j)=>Math.abs(x-wb[j]!)));worldMax=Math.max(worldMax,drift);worldSamples++;assert(drift<1e-8,'World geometry drift '+record.source_id+'/'+a.node+': '+drift);
  const wrong=world(frame.new_world,a.p);negativeDrift=Math.max(negativeDrift,...wa.map((x,j)=>Math.abs(x-wrong[j]!)));
 }
 function rename(v:any):any{if(typeof v==='string')return record.node_aliases[v]??v;if(Array.isArray(v))return v.map(rename);if(v&&typeof v==='object')return Object.fromEntries(Object.entries(v).map(([k,x])=>[record.node_aliases[k]??k,rename(x)]));return v;}
 const owner=(node:string)=>{const id=record.node_destinations[node];assert(id,'Missing destination for '+node);return id;};
 function get(id:string){if(!merged.has(id))merged.set(id,{version:1,collision:record.gameplay.collision,surfaces:[],doors:[],lifts:[],interiors:[],sightOrder:{},draft:{issues:[]}});const g=merged.get(id);assert.equal(g.collision,record.gameplay.collision,'Conflicting collision mode');return g;}
 for(const[id,value]of Object.entries(translated.sightOrder??{})){const g=get(owner(id));const renamed=record.node_aliases[id]??id;assert(g.sightOrder[renamed]===undefined,'Duplicate sightOrder '+renamed);g.sightOrder[renamed]=value;}
 for(const field of arrays){if(!(field in translated))continue;for(const item of (translated as any)[field]){assert(item.node,'Feature without frame '+field);const dst=owner(item.node);const g=get(dst);(g[field]??=[]).push(rename(item));provenance.push({source:record.source_id,destination:dst,field,id:item.id,node:item.node});}}
 for(const dst of new Set(Object.values(record.node_destinations) as string[])){const g=get(dst);g.draft.issues=[...new Set([...g.draft.issues,...(translated.draft?.issues??[])])];for(const f of ['movementBlockers','movementClearances'])if(f in translated)g[f]??=[];}
}
assert(negativeDrift>1,'Negative unshifted migration failed to detect drift');
const outputs:any[]=[];const failures:any[]=[];const totals:Record<string,number>={};
for(const[id,gameplay]of merged){
 const input=inventory.destination_descriptors[id];assert.equal(digest(input),inventory.pins[input]);const descriptor=read(input);assert(!descriptor.gameplay,'Destination already has gameplay');
 for(const field of arrays){const list=gameplay[field]??[];const ids=list.map((x:any)=>x.id);assert.equal(new Set(ids).size,ids.length,'Duplicate '+id+'/'+field);totals[field]=(totals[field]??0)+list.length;}
 descriptor.gameplay=gameplay;
 try{validateAssetGameplay(gameplay,descriptor);}catch(e){failures.push({id,error:String(e)});}
 const file=path.join(out,id+'.json');fs.writeFileSync(file,JSON.stringify(descriptor,null,2)+'\n');outputs.push({id,source_descriptor:input,source_sha256:digest(input),candidate_descriptor:file,candidate_sha256:digest(file),source_parts:descriptor.parts.map((p:any)=>p.node)});
}
for(const[field,count]of Object.entries(inventory.feature_counts))assert.equal(totals[field],count,'Lost features '+field);
const report={status:failures.length?'HOLD schema failures':'PASS private metadata migration',source_inventory_sha256:digest(path.join(root,'frame-inventory.json')),sources:inventory.gameplay_sources,destinations:outputs.length,feature_counts:totals,world_coordinate_samples:worldSamples,max_world_drift:worldMax,max_inverse_roundtrip_error:roundtripMax,negative_unshifted_migration_drift:negativeDrift,failures,outputs,provenance,retirement_candidates:inventory.retirement_candidates,limits:['World gameplay drift tolerance1e-8 uses actual hydrated compiler frames; internal render-node transforms do not redefine metadata frames.','No models, map placements, shared descriptors, indexes or runtime gameplay edited.','Terrain same-ID root gameplay is independently preserved by publisher; these outputs cover superseded groups only.','Existing draft gameplay limitations preserved; migration does not certify full native parity.']};
fs.writeFileSync(path.join(root,'migration-report.json'),JSON.stringify(report,null,2)+'\n');console.log(JSON.stringify({...report,outputs:outputs.length,provenance:provenance.length}));
assert.equal(failures.length,0,'Private candidates require schema fixes');
