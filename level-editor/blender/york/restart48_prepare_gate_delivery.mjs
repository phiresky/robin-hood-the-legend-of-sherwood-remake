/** Prepare an exact-source cropped gate preview with reviewed physical pose samples. */
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {decode,encode} from '../../app/node_modules/fast-png/lib/index.js';
import {validateStateDelivery} from '../../shared/src/state-delivery.ts';
import {verifyNativePresentationSource} from '../../app/src/native-state-presentation.ts';
const root=path.resolve(import.meta.dirname,'../../..'),base=path.join(root,'level-editor/work/york-refinement'),work=path.join(base,'restart2');
const output=path.join(work,process.argv[2]??'restart48-production-gate-delivery-v2');
assert.ok(!fs.existsSync(output),'Use a fresh output directory');fs.mkdirSync(output);
const read=p=>JSON.parse(fs.readFileSync(p)),hash=b=>createHash('sha256').update(b).digest('hex'),pin=p=>({path:p,sha256:hash(fs.readFileSync(p))});
const sourceRoot=path.join(base,'geometry-pass-01/native-state-source-v1'),sourceFile=path.join(sourceRoot,'manifest.json');
assert.equal(pin(sourceFile).sha256,'996e048c12c2326d059c6665ff70bda548937e2d06aa87c8d3f898eee0525cfe');
const manifest=read(sourceFile),lib=path.join(output,'library'),resources=[];
fs.mkdirSync(lib,{recursive:true});
function resource(bytes,ext,source){const sha256=hash(bytes),rel='mission-states/york/resources/'+sha256+'.'+ext,dest=path.join(lib,rel);fs.mkdirSync(path.dirname(dest),{recursive:true});if(!fs.existsSync(dest))fs.writeFileSync(dest,bytes);resources.push({path:rel,sha256,source});return{path:rel,sha256};}
function frames(row,position){return(row?.frames??[]).map(f=>{const file=path.join(sourceRoot,f.image),bytes=fs.readFileSync(file);assert.equal(hash(bytes),f.sha256);return{...resource(bytes,'png',file),width:bytes.readUInt32BE(16),height:bytes.readUInt32BE(20),offset:[f.bbox[0]-position[0],f.bbox[1]-position[1]],delay:f.delay};});}
const levelPath=path.join(root,'level-editor/library/game-data/Data/Levels/york.rhp.json'),missionPath=path.join(root,'level-editor/library/game-data/Data/Levels/S05_Yrk_EC.rhm.json'),level=read(levelPath),mission=read(missionPath);
const origin=[2250,780],width=220,height=250,backgroundPath=path.join(base,'source-states-complete/revealed.png');
const background=decode(fs.readFileSync(backgroundPath)),pixels=new Uint8Array(width*height*4);
assert.ok([3,4].includes(background.channels));
for(let y=0;y<height;y++)for(let x=0;x<width;x++){const from=((y+origin[1])*background.width+x+origin[0])*background.channels,to=(y*width+x)*4;pixels.set(background.data.subarray(from,from+3),to);pixels[to+3]=background.channels===4?background.data[from+3]:255;}
const bg={...resource(encode({width,height,channels:4,data:pixels}),'png',backgroundPath),width,height};
const elements=manifest.records.filter(r=>r.kind==='animation').map((r,index)=>{assert.deepEqual(r.record,level.animations[index]);const pos=[r.sprite.position_x,r.sprite.position_y],center=r.center;return{id:r.id,source:{kind:'map-animation',index,sha256:hash(JSON.stringify(r.record))},active:r.record.active,frames:frames(r.rows[0],pos),loop:true,display_position:pos,sort_position:[pos[0]+center[0],pos[1]+center[1]],display_order:pos[1]+center[1]+r.sprite.elevation,creation_order:level.patches.length+index,polyline:r.record.display_polyline};});
assert.ok(mission.targets.every(t=>!t.polyline?.length),'Target ordering boundaries need explicit binding');
const r=manifest.records.find(r=>r.id==='patch-000'),pos=[r.sprite.position_x,r.sprite.position_y];assert.deepEqual(r.record,level.patches[0]);
const bankPath=path.join(root,'datadirs/fullgame_gog_hackable/Data/Animations/Day/Yorkpatch.rhs.d/manifest.json');assert.equal(pin(bankPath).sha256,r.bank_manifest_sha256);
const profile=read(bankPath).profiles.find(p=>p.name===r.sprite.profile_name);assert.ok(profile);
const initial=frames(r.rows.find(r=>r.action==='PatchInitial'),pos),transition=frames(r.rows.find(r=>r.action==='PatchTransition'),pos),all=[...initial,...transition];
const left=Math.min(...all.map(f=>pos[0]+f.offset[0])),top=Math.min(...all.map(f=>pos[1]+f.offset[1])),right=Math.max(...all.map(f=>pos[0]+f.offset[0]+f.width)),bottom=Math.max(...all.map(f=>pos[1]+f.offset[1]+f.height));
const patch={id:r.id,source:{kind:'map-patch',index:0,sha256:hash(JSON.stringify(r.record))},profile:{...resource(Buffer.from(JSON.stringify(profile)),'json',bankPath),name:profile.name,center:r.center},integrate_in_background:true,activation:'phases',restore_bounds:[left,top,right-left,bottom-top],elevation:r.sprite.elevation,layer:'ordered',display_position:pos,sort_position:[pos[0]+r.center[0],pos[1]+r.center[1]],display_order:pos[1]+r.center[1]+r.sprite.elevation,creation_order:0,polyline:r.record.element_fx.display_polyline,definitive:true,initial,transition,final:[],initial_loop:true,final_loop:false};
const exportBase=path.join(work,'restart44-approved-portcullis-export-v1');
const position=[2357.217041015625,109.8709487915039,1801.3960571289062];
function endpoint(state,expected){const model=path.join(exportBase,state,'3d-assets/york-castle-portcullis/model.glb');assert.equal(pin(model).sha256,expected);const relative='3d-assets/york/york-portcullis-'+state+'/model.glb',dest=path.join(lib,relative);fs.mkdirSync(path.dirname(dest),{recursive:true});fs.linkSync(model,dest);const raw=fs.readFileSync(model),gltf=JSON.parse(raw.subarray(20,20+raw.readUInt32LE(12)).toString());return[{id:'york-portcullis-'+state,role:'objects',model:relative,model_sha256:expected,model_scene:gltf.scenes[gltf.scene??0].name,resources:[],position}];}
const motionPath=path.join(work,'jamb-clearance-candidate-v1/motion-proposal.json');assert.equal(pin(motionPath).sha256,'07fd102ae0a508bf77ff10f6d52dda6c746723d3da7011b312b9164b7758a446');const motion=read(motionPath);
const rows=motion.rows;
assert.equal(rows?.length,45);
const contract={version:1,scope:'controlled-state-preview',native:{version:1,mission:'S05_Yrk_EC',mission_data_sha256:hash(JSON.stringify(mission)),level_data_sha256:hash(JSON.stringify(level)),camera_elevation_deg:35,scope:'map-art-and-listed-effects',background:bg,origin,elements,patch_states:[patch]},families:[{id:'york-portcullis',element_ids:[],background_ids:[],patch_ids:['patch-000'],body_terminal_tick:89,physical:{initial:endpoint('covered','fd7429ff5cea5ff35dee1a2c063345f63d153f11e43d21e74190d94a26759c30'),applied:endpoint('raised','9b4fb94168bf95459a958f81a0a8a5e05e06c2042d0f59b4b36df7c1fc58a226'),motion:{kind:'native-patch-translation',patch_id:'patch-000',source_motion_sha256:pin(motionPath).sha256,translation_frames:rows.map(r=>[0,r.nominal_lift_world_z,0])}}}]};
validateStateDelivery(contract);await verifyNativePresentationSource(contract.native,{name:'S05_Yrk_EC',data:mission,level,camera:{kind:'oblique-orthographic',elevation_deg:35}});
const contractPath=path.join(lib,'mission-states/york/gate.json');fs.writeFileSync(contractPath,JSON.stringify(contract,null,2)+'\n');
fs.writeFileSync(path.join(output,'handoff.json'),JSON.stringify({status:'SOURCE_BOUND_PRODUCTION_CONTRACT_PRIVATE',contract:pin(contractPath),library:lib,source:{name:'S05_Yrk_EC',mission:pin(missionPath),level:pin(levelPath)},pins:[pin(sourceFile),pin(motionPath),pin(backgroundPath),pin(bankPath)],resources,scope:'Cropped gate artwork and exact approved moving portcullis. All map animation source bindings retained. Other patch states, mission target visuals, door/collision/audio gameplay and full map native presentation are excluded. Frame36 remains approved inference.'},null,2)+'\n');
console.log(contractPath);
