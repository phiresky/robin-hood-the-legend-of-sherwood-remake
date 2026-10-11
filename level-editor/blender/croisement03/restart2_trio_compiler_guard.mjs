/** Freeze source and input pins around same-process gameplay comparisons. */
import fs from 'node:fs/promises';import path from 'node:path';import crypto from 'node:crypto';import assert from 'node:assert/strict';
import {readStoredMap,pinnedDescriptors} from '../../pipeline/src/stored-map.ts';import {compileAssetGameplay} from '../../shared/src/compile-asset-gameplay.ts';
export const hash=b=>crypto.createHash('sha256').update(b).digest('hex');
export async function pins(paths){return Object.fromEntries(await Promise.all(paths.map(async p=>[path.resolve(p),hash(await fs.readFile(p))])));}
export async function verifyPins(records){for(const[p,h]of Object.entries(records))assert.equal(hash(await fs.readFile(p)),h,'Pinned compiler/input changed: '+p);}
export async function compilerPins(){
 const base=path.resolve('level-editor');const shared=path.join(base,'shared/src');const files=(await fs.readdir(shared)).filter(n=>n.endsWith('.ts')&&!n.endsWith('.test.ts')).map(n=>path.join(shared,n));
 files.push(path.join(base,'pipeline/src/stored-map.ts'),path.join(base,'pipeline/src/scene-assets.ts'),path.join(base,'app/src/scenery-resources.ts'),path.join(base,'app/src/scenery-manifest.ts'),path.join(base,'pnpm-lock.yaml'),path.resolve('level-editor/blender/croisement03/restart2_trio_compiler_guard.mjs'));
 return pins(files);
}
export async function compilePinned(file,library,{deduplicateApprovedDrafts=false}={}){
 const raw=JSON.parse(await fs.readFile(file));const inputPins=await pins([file,...[...raw.assetSources,...raw.sceneAssets].filter(r=>r.descriptor).map(r=>path.join(library,r.descriptor))]);
 const doc=await readStoredMap(file,library),assets=await pinnedDescriptors(library,doc.assetSources,doc.sceneAssets),repairs=[];
 if(deduplicateApprovedDrafts)for(const id of ['croisement03-east-tree-rocks','croisement03-southeast-stone-wall']){
  const descriptor=assets.get(id);assert.ok(descriptor,'Missing declared draft repair '+id);const before=structuredClone(descriptor.gameplay.draft.issues),after=[...new Set(before)];assert.ok(before.length>after.length);descriptor.gameplay.draft.issues=after;repairs.push({id,before,after});
 }
 let scenerySources=[];const geometry=compileAssetGameplay(doc,assets,[0,0,1408,960],{onProgress:s=>console.log(s),onSceneryCompiled:s=>scenerySources=s});await verifyPins(inputPins);
 return {geometry:JSON.parse(JSON.stringify(geometry)),scenerySources,inputPins,repairs};
}
