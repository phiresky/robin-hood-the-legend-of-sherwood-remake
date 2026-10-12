/** Check final staged hall through the normal catalog loader, without browser rendering. */
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {loadMissionStateCatalog,loadMissionStatePreview} from '../../app/src/mission-state-catalog.ts';
const root=path.resolve(import.meta.dirname,'../../..'),stage=path.join(root,'level-editor/work/active/york/restart2/restart72-hall-normal-catalog-stage-v1'),lib=path.join(stage,'library');
function dir(base){return{kind:'directory',name:path.basename(base),async getDirectoryHandle(name){const next=path.join(base,name);if(!fs.statSync(next,{throwIfNoEntry:false})?.isDirectory())throw new DOMException('Absent','NotFoundError');return dir(next);},async getFileHandle(name){const file=path.join(base,name);if(!fs.statSync(file,{throwIfNoEntry:false})?.isFile())throw new DOMException('Absent','NotFoundError');return{kind:'file',name,async getFile(){return new File([fs.readFileSync(file)],name);}};}};}
const entries=await loadMissionStateCatalog(dir(lib),'york','S05_Yrk_EC'),entry=entries.find(e=>e.id==='york-hall000-reviewed-loop');assert.ok(entry);assert.equal(entries.filter(e=>e.id===entry.id).length,1);
const delivery=await loadMissionStatePreview(dir(lib),entry);assert.equal(delivery.kind,'native-loop');assert.equal(delivery.contract.cover_visibility[0].source.index,2);assert.equal(delivery.contract.native.cover_artwork.variants.length,4);assert.equal(delivery.contract.physical.targets[0].source.model_sha256,'3edab8b9da5fd995205224e956c5a76d716eaaa49d7d3b3380df04e0dbf99eba');
const plan=JSON.parse(fs.readFileSync(path.join(stage,'plan.json')));for(const r of plan.records)assert.equal(createHash('sha256').update(fs.readFileSync(r.source)).digest('hex'),r.sha256);
const views=JSON.parse(fs.readFileSync(path.join(stage,'saved-hall-views.json'))).views;assert.equal(views.length,8);assert.equal(views[0].azimuth_degrees,0);
fs.writeFileSync(path.join(stage,'browser-config.json'),JSON.stringify({entry,views,installed_hall:delivery.contract.cover_visibility[0].placement,check_script:'level-editor/blender/york/restart72_hall_catalog_ui_check.js',capture_handshake:'window.scatterStage -> owner screenshot -> window.scatterCaptures[name]'},null,2)+'\n');
const report={status:'PASS_NORMAL_CATALOG_LOADER',entry,resources_verified:plan.records.length,cover_background_states:4,cover_effect_source_index:2,source_clock_ticks:18,browser_run:false};fs.writeFileSync(path.join(stage,'catalog-loader-report.json'),JSON.stringify(report,null,2)+'\n');console.log(JSON.stringify(report));
