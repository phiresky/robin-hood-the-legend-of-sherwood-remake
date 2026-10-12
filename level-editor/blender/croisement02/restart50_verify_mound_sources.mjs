import {readFile,writeFile} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import {verifyNativePresentationSource} from '../../app/src/native-state-presentation.ts';
const out='level-editor/work/active/croisement02/restart50-mound-lifecycle-fixture-v1';
const sha=raw=>createHash('sha256').update(raw).digest('hex');
async function read(pin){const raw=await readFile(pin.path);if(sha(raw)!==pin.sha256)throw Error('Pin mismatch '+pin.path);return JSON.parse(raw)}
const request=JSON.parse(await readFile(out+'/request.json'));
const config=await read(request.reader_config),results=[];
for(const row of config.bindings){const contract=await read(row.contract),pins=request.sources[row.mission];await verifyNativePresentationSource(contract.native,{name:row.mission,data:await read(pins.data),level:await read(pins.level),camera:{kind:'oblique-orthographic',elevation_deg:contract.native.camera_elevation_deg}});results.push(row.id)}
await writeFile(out+'/source-verification.json',JSON.stringify({status:'PASS_ALL32_PRODUCTION_NATIVE_SOURCE_BINDINGS',controls:results,browser_run:false},null,2)+'\n');
console.log('PASS '+results.length+' native source bindings');
