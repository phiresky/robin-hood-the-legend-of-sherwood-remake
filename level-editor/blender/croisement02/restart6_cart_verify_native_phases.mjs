import {readFile,writeFile} from 'node:fs/promises';import {resolve,join} from 'node:path';import {createHash} from 'node:crypto';
import {NativeStatePresentation} from '../../app/src/native-state-presentation.ts';
const out=resolve('level-editor/work/croisement02-refinement/restart2-state/cart-contract-preparation-v2'),library=resolve('level-editor/library'),read=async p=>JSON.parse(await readFile(p,'utf8')),manifest=await read(join(out,'manifest.json')),resources=new Map(manifest.resources.map(r=>[r.path,r.source])),level=await read(join(library,'game-data/Data/Levels/Croisement02.rhp.json')),records=[];
for(const r of manifest.records){if(!r.contract)continue;const contract=await read(join(out,r.contract)),family=contract.families[0],mission=await read(join(library,'game-data/Data/Levels',r.mission+'.rhm.json')),player=new NativeStatePresentation();
try{await player.set(contract.native,{name:r.mission,data:mission,level,camera:{kind:'oblique-orthographic',elevation_deg:35}},f=>readFile(resources.get(f.path)??join(library,f.path)));const cases=[{tick:-1,rgba_sha256:createHash('sha256').update(player.pixels().data).digest('hex')}];
for(let tick=0;tick<=family.body_terminal_tick+1;tick++){player.seek(tick);for(const id of family.element_ids)player.setElementState(id,true,tick);cases.push({tick,rgba_sha256:createHash('sha256').update(player.pixels().data).digest('hex')});}
records.push({contract:r.contract,status:'COMPOSED',cases});console.log(r.contract,cases.length);
}catch(e){records.push({contract:r.contract,status:'HOLD',error:String(e)});}finally{player.dispose();}}
await writeFile(join(out,'runtime-phase-verification.json'),JSON.stringify({scope:'Current shared runtime with source resource hashes verified; independent comparison separate.',records},null,2)+'\n');
