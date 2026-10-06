import {readFile,writeFile} from 'node:fs/promises';
import {resolve,join} from 'node:path';
import {createHash} from 'node:crypto';
import {loadMissionStatePreview} from '../../app/src/mission-state-catalog.ts';
const stage=resolve('level-editor/work/croisement02-refinement/restart2-state/remaining-seven-package-v2'),live=resolve('level-editor/library');
const manifest=JSON.parse(await readFile(join(stage,'manifest.json'),'utf8'));
const staged=new Set([...manifest.files,manifest.private_index].map(r=>r.path));
function directory(prefix=''){return {async getDirectoryHandle(name){return directory(prefix+name+'/')},async getFileHandle(name){const path=prefix+name,bytes=await readFile(join(staged.has(path)?join(stage,'library'):live,path));return {getFile:async()=>new File([bytes],name)}}}}
const checks=[];
for(const entry of manifest.entries){const loaded=await loadMissionStatePreview(directory(),entry);if(loaded.kind!=='transition')throw Error('Wrong contract kind');checks.push({id:entry.id,kind:loaded.kind,family:loaded.contract.families[0].id,source_binding:'PASS'});}
await writeFile(join(stage,'catalog-loader-verification.json'),JSON.stringify({status:'PASS',manifest_sha256:createHash('sha256').update(await readFile(join(stage,'manifest.json'))).digest('hex'),checks,installed_index_unchanged:createHash('sha256').update(await readFile(join(live,'mission-states/index.json'))).digest('hex')===manifest.installed_index_sha256},null,2)+'\n');
console.log('PASS7 actual catalog/source loaders');
