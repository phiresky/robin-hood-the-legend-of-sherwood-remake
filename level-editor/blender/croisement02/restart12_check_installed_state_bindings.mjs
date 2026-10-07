// Read-only current-library binding proof; state publication remains a separate transaction.
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {resolve,join} from 'node:path';
import {createHash} from 'node:crypto';
import {parseStoredMap} from '../../shared/src/stored-level.ts';
import {verifyStaticStateReplacements} from '../../shared/src/state-delivery.ts';
const [expected,outArg]=process.argv.slice(2);
if(!/^[0-9a-f]{64}$/.test(expected??'')||!outArg)throw Error('Expected exact map SHA256 and fresh output directory');
const root=resolve('.'),library=join(root,'level-editor/library'),base=join(root,'level-editor/work/croisement02-refinement/restart2-state');
const stage=join(base,'remaining-seven-package-v2'),out=resolve(outArg),mapPath=join(library,'scenes/croisement02.rhlos-map.json');
if(!out.startsWith(base+'/'))throw Error('Output must remain in private state workspace');
const sha=bytes=>createHash('sha256').update(bytes).digest('hex'),read=async path=>JSON.parse(await readFile(path));
const bytes=await readFile(mapPath);if(sha(bytes)!==expected)throw Error('Current map differs from requested frozen map');
const raw=JSON.parse(bytes),manifestBytes=await readFile(join(stage,'manifest.json')),manifest=JSON.parse(manifestBytes),plan=await read(join(base,'remaining-seven-publication-v1/plan.json'));
if(sha(manifestBytes)!==plan.manifest.sha256)throw Error('State manifest differs from publication plan');
const resources=[],descriptors=new Map();
for(const ref of [...raw.assetSources,...raw.sceneAssets]){
 for(const field of ['model','descriptor']){
  const actual=sha(await readFile(join(library,ref[field])));
  if(actual!==ref[field+'_sha256'])throw Error('Installed resource differs: '+ref.id+' '+field);
  resources.push({path:ref[field],sha256:actual});
 }
 descriptors.set(ref.id,await read(join(library,ref.descriptor)));
}
const document=parseStoredMap(raw,descriptors),checks=[];
for(const entry of manifest.entries){
 const path=join(stage,'library',entry.contract.path),contractBytes=await readFile(path);
 if(sha(contractBytes)!==entry.contract.sha256)throw Error('Contract resource differs: '+entry.id);
 const contract=JSON.parse(contractBytes);verifyStaticStateReplacements(contract,document);
 checks.push({id:entry.id,status:'PASS',contract_sha256:sha(contractBytes),replacement_parts:contract.families.flatMap(f=>Object.values(f.static_replacements??{}).flat().map(p=>p.object_id))});
}
if(checks.length!==7||plan.added_ids.some(id=>!checks.some(c=>c.id===id)))throw Error('Seven exact planned bindings required');
if(sha(await readFile(mapPath))!==expected)throw Error('Map changed during verification');
for(const row of resources)if(sha(await readFile(join(library,row.path)))!==row.sha256)throw Error('Resource changed during verification');
const runtimePins={};for(const name of ['stored-level.ts','state-delivery.ts']){const path=join(root,'level-editor/shared/src',name);runtimePins[path.slice(root.length+1)]=sha(await readFile(path))}
const report={status:'PASS',scope:'Read-only seven exact contract bindings against currently installed static map; no approval of physical motion or publication.',map_sha256:expected,contract_manifest_sha256:sha(manifestBytes),plan_sha256:sha(await readFile(join(base,'remaining-seven-publication-v1/plan.json'))),checks,source_resources:resources,runtime_pins:runtimePins,publication:false};
await mkdir(out,{recursive:false});await writeFile(join(out,'verification.json'),JSON.stringify(report,null,2)+'\n');console.log(JSON.stringify({status:'PASS',checks:checks.length,source_resources:resources.length,map_sha256:expected,out}));
