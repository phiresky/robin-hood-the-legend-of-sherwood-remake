import * as THREE from 'three';
import { createGltfLoader } from '../../app/src/gltf-loader.ts';
import { MissionStateLayer, missionStateDataHash } from '../../app/src/mission-state-layer.ts';
import { retainSceneAnimations } from '../../app/src/scene-assets.ts';

const renderer = new THREE.WebGLRenderer({antialias:true, preserveDrawingBuffer:true});
renderer.setSize(512,512); renderer.setPixelRatio(1); renderer.setClearColor(0x323232);
document.body.append(renderer.domElement);
const scene = new THREE.Scene();
const camera = new THREE.OrthographicCamera(-80,80,80,-80,.1,20000);
const manager = new THREE.LoadingManager();
const checkedResources = new Map<string,string>();
manager.setURLModifier(value=>checkedResources.get(new URL(value,location.href).pathname)||value);
const loader = createGltfLoader(manager);
const status: any = {loaded:false, stage:'manifest', errors:[]};
(window as any).physicalSigns = status;
const url = (file: string) => '/@fs/'+file;
async function checked(file: string, expected: string) {
  const response = await fetch(url(file)); if (!response.ok) throw Error('Fetch '+file);
  const bytes = await response.arrayBuffer();
  const hash = Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',bytes)),n=>n.toString(16).padStart(2,'0')).join('');
  if(hash!==expected) throw Error('Changed source '+file);
  return bytes;
}
const json = async (r: any) => JSON.parse(new TextDecoder().decode(await checked(r.file,r.sha256)));
try {
  const manifest = await (await fetch('/manifest.json')).json();
  const mission = await json(manifest.mission), level = await json(manifest.level);
  const signBytes = await checked(manifest.model.file,manifest.model.sha256);
  const sourceReference = {id:'croisement02-mission-rotating-sign',role:'objects',model:'physical-signs/animated-sign.glb',model_sha256:manifest.model.sha256,resources:[]};
  const contract = {version:1,mission:'S03_FoB_MP',mission_data_sha256:await missionStateDataHash(mission),
    level_data_sha256:await missionStateDataHash(level),camera_elevation_deg:35,
    targets:await Promise.all(manifest.instances.map(async (row: any)=>({id:'sign-'+row.target_index,target_index:row.target_index,
      target_sha256:await missionStateDataHash(mission.targets[row.target_index]),source:sourceReference,model_origin:[0,0,0],representation:'physical',
      actions:[0,210,211].map(action=>({action,clip:manifest.model.clip,timing:{mode:'loop',cycleTicks:64}}))})))};
  let replacements: number[] = [];
  const layer = new MissionStateLayer(targets=>{replacements=[...targets].sort()},message=>status.errors.push(message),
    ()=>{
      const pending=loader.parseAsync(signBytes.slice(0),'').then(parsed=>{
        retainSceneAnimations(parsed.scene,parsed.animations,parsed.scenes);return parsed.scene;
      });
      return {load:async()=>pending,dispose(){}};
    });
  const source = {name:'S03_FoB_MP',data:mission,level,camera:manifest.camera};
  await layer.set(contract as any,{} as FileSystemDirectoryHandle,source);
  if(layer.players.size!==5||status.errors.length)throw Error('Layer did not load five signs '+JSON.stringify(status.errors));
  scene.add(layer.root);
  const resources=await(await fetch('/resources-v1.json')).json();
  const contexts: any[]=[];
  for(const row of manifest.context) {
    status.stage='context '+row.id;
    for(const resource of resources.assets.find((r:any)=>r.id===row.id).resources){
      const bytes=await checked(resource.file,resource.sha256);
      checkedResources.set(url(resource.file),URL.createObjectURL(new Blob([bytes])));
    }
    const asset=await loader.parseAsync(await checked(row.file,row.sha256),url(row.file.slice(0,row.file.lastIndexOf('/')+1)));
    const wrapper=new THREE.Group(); wrapper.matrix.fromArray(row.world_matrix);wrapper.matrixAutoUpdate=false;
    wrapper.add(asset.scene);scene.add(wrapper);
    asset.scene.traverse(node=>{if(row.placement.parts?.[node.name]?.hidden)node.visible=false});
    contexts.push({row,object:wrapper});
  }
  const sine=Math.sin(35*Math.PI/180),cosine=Math.cos(35*Math.PI/180);
  function states() {
    return [...layer.players].map(([id,{player}])=>{
      const nodes: any[]=[];
      player.content.traverse(o=>{
        if(Number.isInteger(o.userData.native_body_frame)||Number.isInteger(o.userData.native_frame))
          nodes.push({uuid:o.uuid,name:o.name,body:o.userData.native_body_frame??null,shadow:o.userData.native_frame??null,scale:o.scale.toArray()});
      });
      return {id,tick:player.tick,position:player.object.position.toArray(),active:nodes.filter(n=>n.scale.some((v:number)=>v>.5)),nodeUUIDs:nodes.map(n=>n.uuid)};
    });
  }
  function seek(tick:number) {for(const [id]of layer.players)layer.seek(id,tick);}
  function view(target:number,angle=0) {
    const row=manifest.instances.find((r:any)=>r.target_index===target);
    const player=layer.players.get('sign-'+target)!.player;
    const center=player.object.position.clone().add(new THREE.Vector3(0,16,0));
    const direction=new THREE.Vector3(Math.sin(angle)*cosine,sine,Math.cos(angle)*cosine);
    camera.position.copy(center).addScaledVector(direction,4000);camera.lookAt(center);camera.updateMatrixWorld(true);
    // The complete conservative context is retained at both viewing angles.
    for(const c of contexts)c.object.visible=c.row.near_targets.includes(target);
    renderer.render(scene,camera);
    return {target,angle,camera:camera.matrixWorld.toArray(),position:player.object.position.toArray(),png:renderer.domElement.toDataURL('image/png')};
  }
  status.grounding=()=>manifest.instances.map((row:any)=>{
    const position=layer.players.get('sign-'+row.target_index)!.player.object.position.clone();
    scene.updateMatrixWorld(true);
    const ray=new THREE.Raycaster(position.clone().add(new THREE.Vector3(0,500,0)),new THREE.Vector3(0,-1,0),0,1000);
    const candidates=contexts.filter(c=>c.row.near_targets.includes(row.target_index)&&(/ground|bank|rock/.test(c.row.id)||c.row.source.role==='ground'));
    const hits=candidates.flatMap(c=>ray.intersectObject(c.object,true).map(h=>({asset:c.row.id,object:h.object.name,point:h.point.toArray(),distance:h.distance,face:h.faceIndex})));
    hits.sort((a,b)=>a.distance-b.distance);
    return {target:row.target_index,anchor:position.toArray(),surface_hits:hits.slice(0,12),note:'Vertical geometric intersections with terrain/context; texture alpha is not evaluated.'};
  });
  status.omission=(target:number,omit:string)=>{
    seek(0);view(target,0);
    for(const c of contexts)if(omit==='all'||c.row.id===omit)c.object.visible=false;
    const oldColor=renderer.getClearColor(new THREE.Color()),oldAlpha=renderer.getClearAlpha();
    if(omit==='all')renderer.setClearColor(0,0);
    renderer.render(scene,camera);
    const png=renderer.domElement.toDataURL('image/png');renderer.setClearColor(oldColor,oldAlpha);
    return {target,omit,png,camera:camera.matrixWorld.toArray()};
  };
  status.contract=contract;status.manifest=manifest;status.layer=layer;
  status.states=states;status.seek=seek;status.view=view;
  status.replacements=()=>replacements;
  status.clear=()=>layer.clear();
  status.reset=async()=>{await layer.set(contract as any,{} as FileSystemDirectoryHandle,source);return states()};
  status.advance=(seconds:number)=>{layer.setPlaying(true);layer.advance(seconds);layer.setPlaying(false);return states()};
  seek(0);view(4);status.loaded=true;status.stage='ready';
} catch(error) {status.error=String(error);status.failedStage=status.stage;status.stage='failed';console.error(error)}
