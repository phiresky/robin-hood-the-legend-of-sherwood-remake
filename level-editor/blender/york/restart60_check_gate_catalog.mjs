/** Check final staged gate through the normal catalog loader, without browser rendering. */
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {loadMissionStateCatalog,loadMissionStatePreview} from '../../app/src/mission-state-catalog.ts';
const root=path.resolve(import.meta.dirname,'../../..'),stage=path.join(root,'level-editor/work/active/york/restart2/restart57-gate-catalog-stage-v1'),lib=path.join(stage,'library');
function dir(base){return{kind:'directory',name:path.basename(base),async getDirectoryHandle(name){const next=path.join(base,name);if(!fs.statSync(next,{throwIfNoEntry:false})?.isDirectory())throw new DOMException('Absent','NotFoundError');return dir(next);},async getFileHandle(name){const file=path.join(base,name);if(!fs.statSync(file,{throwIfNoEntry:false})?.isFile())throw new DOMException('Absent','NotFoundError');return{kind:'file',name,async getFile(){return new File([fs.readFileSync(file)],name);}};}};}
const entries=await loadMissionStateCatalog(dir(lib),'york','S05_Yrk_EC');assert.equal(entries.length,1);assert.equal(entries[0].id,'york-portcullis');
const delivery=await loadMissionStatePreview(dir(lib),entries[0]);assert.equal(delivery.kind,'transition');assert.equal(delivery.contract.families[0].physical.motion.translation_frames.length,45);assert.equal(delivery.contract.families[0].body_terminal_tick,89);
const old=JSON.parse(fs.readFileSync(path.join(root,'level-editor/work/active/york/restart2/restart48-production-gate-delivery-v2/library/mission-states/york/gate.json'))),current=structuredClone(delivery.contract);
for(const endpoint of ['initial','applied'])current.families[0].physical[endpoint][0].model=old.families[0].physical[endpoint][0].model;
assert.deepEqual(current,old,'Final relocation changed approved contract semantics');
const report={status:'PASS_NORMAL_CATALOG_LOADER',entry:entries[0],transition_frames:45,terminal_tick:89,only_model_paths_changed:true,day_only:true,scope:'Actual catalog parser/loader and source binding. Browser editor interaction is separate.'};fs.writeFileSync(path.join(stage,'catalog-loader-report.json'),JSON.stringify(report,null,2)+'\n');console.log(JSON.stringify({status:report.status,entry:entries[0].id}));
