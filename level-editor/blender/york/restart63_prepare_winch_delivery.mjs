/** Bind the complete mechanism to its independent native patch clock and transparent initial. */
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {validateStateDelivery} from '../../shared/src/state-delivery.ts';
import {verifyNativePresentationSource} from '../../app/src/native-state-presentation.ts';

const root=path.resolve(import.meta.dirname,'../../..'),base=path.join(root,'level-editor/work/york-refinement'),work=path.join(base,'restart2');
const [exportDir,exportSha,outputDir]=process.argv.slice(2);
assert.ok(exportDir&&exportSha&&outputDir,'Usage: <export dir> <export report sha256> <fresh output dir>');
const out=path.resolve(outputDir),exp=path.resolve(exportDir);
assert.ok(out.startsWith(work+'/')&&exp.startsWith(work+'/'));assert.ok(!fs.existsSync(out));
const hash=bytes=>createHash('sha256').update(bytes).digest('hex'),sha=p=>hash(fs.readFileSync(p)),read=p=>JSON.parse(fs.readFileSync(p));
assert.equal(sha(path.join(exp,'export-report.json')),exportSha);
const exported=read(path.join(exp,'export-report.json'));
assert.ok(exported.all45poses_geometry_exact);
const guard=read(path.join(exp,'source-surface-guard.json'));
assert.equal(guard.exported_glb_sha256,exported.model_sha256);assert.equal(guard.objects.length,109);
const poseFile=path.join(work,'restart62-winch-rigid-poses-v2/poses.json');assert.equal(sha(poseFile),'c277b27992bebb1eb0f29455ea019af1b41efa905ae7b39a59c9bf83b694330c');
const poses=read(poseFile);
const originalLib=path.join(work,'restart48-production-gate-delivery-v2/library'),contract=read(path.join(originalLib,'mission-states/york/gate.json'));
assert.equal(sha(path.join(originalLib,'mission-states/york/gate.json')),'1cb1afa63b3e52d64d8480fc6e9305db197b53f0f1b410e49b73e45e1448575d');
const sourceRoot=path.join(base,'geometry-pass-01/native-state-source-v1'),manifestPath=path.join(sourceRoot,'manifest.json');
assert.equal(sha(manifestPath),'996e048c12c2326d059c6665ff70bda548937e2d06aa87c8d3f898eee0525cfe');
const record=read(manifestPath).records.find(r=>r.id==='patch-004');
const lib=path.join(out,'library');fs.mkdirSync(lib,{recursive:true});
const pins=[];
function link(file,relative,expected){assert.equal(sha(file),expected);const dest=path.join(lib,relative);fs.mkdirSync(path.dirname(dest),{recursive:true});if(!fs.existsSync(dest))fs.linkSync(file,dest);else assert.equal(sha(dest),expected);pins.push({path:relative,sha256:expected});return{path:relative,sha256:expected};}
function resource(file){return link(file,'mission-states/york/resources/'+sha(file)+path.extname(file),sha(file));}
for(const r of [contract.native.background,...contract.native.elements.flatMap(e=>e.frames)])link(path.join(originalLib,r.path),r.path,r.sha256);
const position=[record.sprite.position_x,record.sprite.position_y];
function frames(action){return(record.rows.find(r=>r.action===action)?.frames??[]).map(f=>{
 const file=path.join(sourceRoot,f.image);assert.equal(sha(file),f.sha256);const bytes=fs.readFileSync(file);
 return{...resource(file),width:bytes.readUInt32BE(16),height:bytes.readUInt32BE(20),offset:[f.bbox[0]-position[0],f.bbox[1]-position[1]],delay:f.delay};
});}
const bank=path.join(root,'datadirs/fullgame_gog_hackable/Data/Animations/Day/Yorkpatch.rhs.d/manifest.json');assert.equal(sha(bank),record.bank_manifest_sha256);
const profile=read(bank).profiles.find(p=>p.name===record.sprite.profile_name);assert.ok(profile);
const profileBytes=Buffer.from(JSON.stringify(profile)),profileSha=hash(profileBytes),profileRel='mission-states/york/resources/'+profileSha+'.json';fs.mkdirSync(path.dirname(path.join(lib,profileRel)),{recursive:true});fs.writeFileSync(path.join(lib,profileRel),profileBytes);pins.push({path:profileRel,sha256:profileSha});
assert.equal(record.rows.find(r=>r.action==='PatchInitial').frames[0].sha256,'713571243237ee36500dd660ad85ceb8a65923e4c33fbd4aac353b2cb860d46d');
const initial=frames('PatchInitial'),transition=frames('PatchTransition');assert.equal(initial.length,1);assert.equal(transition.length,45);assert.equal(initial[0].width,1);assert.equal(initial[0].height,1);
const all=[...initial,...transition],left=Math.min(...all.map(f=>position[0]+f.offset[0])),top=Math.min(...all.map(f=>position[1]+f.offset[1])),right=Math.max(...all.map(f=>position[0]+f.offset[0]+f.width)),bottom=Math.max(...all.map(f=>position[1]+f.offset[1]+f.height));
assert.ok(left>=contract.native.origin[0]&&top>=contract.native.origin[1]&&right<=contract.native.origin[0]+contract.native.background.width&&bottom<=contract.native.origin[1]+contract.native.background.height);
contract.native.patch_states=[{id:'patch-004',source:{kind:'map-patch',index:4,sha256:hash(JSON.stringify(record.record))},profile:{path:profileRel,sha256:profileSha,name:profile.name,center:record.center},integrate_in_background:true,activation:'phases',restore_bounds:[left,top,right-left,bottom-top],elevation:record.sprite.elevation,layer:record.sprite.elevation===0?'background':'ordered',display_position:position,sort_position:[position[0]+record.center[0],position[1]+record.center[1]],display_order:position[1]+record.center[1]+record.sprite.elevation,creation_order:4,polyline:record.record.element_fx.display_polyline,definitive:true,initial,transition,final:[],initial_loop:true,final_loop:false}];
const model=path.join(exp,'3d-assets/york-castle-winch/model.glb'),relative='mission-states/york/models/winch-'+exported.model_sha256.slice(0,16)+'.glb';link(model,relative,exported.model_sha256);
const raw=fs.readFileSync(model),gltf=JSON.parse(raw.subarray(20,20+raw.readUInt32LE(12)).toString());
const pivot=poses.source_origin_scene;
contract.families=[{id:'york-winch',element_ids:[],background_ids:[],patch_ids:['patch-004'],body_terminal_tick:89,physical:{initial:{kind:'absent'},applied:[{id:'york-winch-applied',role:'objects',model:relative,model_sha256:exported.model_sha256,model_scene:gltf.scenes[gltf.scene??0].name,resources:[],position:[pivot[0],pivot[2],-pivot[1]]}],motion:{kind:'native-patch-node-poses',patch_id:'patch-004',source_motion_sha256:sha(poseFile),node_frames:poses.frames}}}];
const missionPath=path.join(root,'level-editor/library/game-data/Data/Levels/S05_Yrk_EC.rhm.json'),levelPath=path.join(root,'level-editor/library/game-data/Data/Levels/york.rhp.json');
const mission=read(missionPath),level=read(levelPath);assert.equal(mission.header.ambiance,1);assert.deepEqual(record.record,level.patches[4]);validateStateDelivery(contract);await verifyNativePresentationSource(contract.native,{name:'S05_Yrk_EC',data:mission,level,camera:{kind:'oblique-orthographic',elevation_deg:35}});
const contractRel='mission-states/york/contracts/winch-day-v1.json',contractFile=path.join(lib,contractRel);fs.mkdirSync(path.dirname(contractFile),{recursive:true});fs.writeFileSync(contractFile,JSON.stringify(contract)+'\n');
const entry={id:'york-winch',name:'Castle winch — reviewed mechanism',kind:'transition',map:'york',mission:'S05_Yrk_EC',contract:{path:contractRel,sha256:sha(contractFile)},mission_data:link(missionPath,'mission-states/york/source/'+path.basename(missionPath),sha(missionPath)),level_data:link(levelPath,'mission-states/york/source/'+path.basename(levelPath),sha(levelPath))};
fs.writeFileSync(path.join(lib,'mission-states/index.json'),JSON.stringify({version:1,entries:[entry]})+'\n');
fs.writeFileSync(path.join(out,'handoff.json'),JSON.stringify({status:'SOURCE_BOUND_PRIVATE_MECHANISM_PENDING_APPROVAL_AND_BROWSER',entry,export_report_sha256:exportSha,source_surface_guard_sha256:sha(path.join(exp,'source-surface-guard.json')),pose_data_sha256:sha(poseFile),resources:pins,canonical_writes:false,scope:'Independent patch004 mechanism only. Room005/gate000 not activated or hidden by this contract. Appearance approval and actual browser/editor proof remain mandatory.'},null,2)+'\n');
console.log(contractFile);
