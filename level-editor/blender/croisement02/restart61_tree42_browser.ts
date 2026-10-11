/** Private saved-model morph playback checks through the production loader/player. */
import type * as THREE from 'three';
import { SceneAssetLoader, captureLoadedStateAppearance } from '../../app/src/scene-assets.ts';
import { StateAppearancePlayer } from '../../app/src/state-appearance-player.ts';
import { TextureDisplay } from '../../app/src/texture-display.ts';
type Pin={path:string;sha256:string};
type Request={model:Pin;placement:{dx:number;dy:number;dz:number;rot_deg:number}};
type Hooks={phase?:(tick:number,object:THREE.Object3D)=>Promise<void>|void};
export async function verifyTree42(request:Request,hooks:Hooks={}){
 const checks:{name:string;pass:boolean}[]=[];const check=(name:string,pass:boolean)=>{checks.push({name,pass});if(!pass)throw Error(name)};
 const response=await fetch('/@fs'+request.model.path);check('Pinned model HTTP',response.ok);const bytes=await response.arrayBuffer();const digest=[...new Uint8Array(await crypto.subtle.digest('SHA-256',bytes))].map(x=>x.toString(16).padStart(2,'0')).join('');check('Pinned model hash',digest===request.model.sha256);
 const root={async getDirectoryHandle(){return root},async getFileHandle(name:string){check('Only embedded model requested',name==='tree42.glb');return{async getFile(){return new File([bytes],name)}}}} as unknown as FileSystemDirectoryHandle;
 const loader=new SceneAssetLoader(root);const loaded=await loader.load({id:'tree42-approved-motion',role:'objects',model:'tree42.glb',model_sha256:digest,resources:[]});const template=captureLoadedStateAppearance(loaded);check('Loaded morph clip captured',!!template);
 const player=new StateAppearancePlayer(template!);const display=new TextureDisplay();display.apply(player.object,1);const p=request.placement;check('Approved placement unrotated',p.rot_deg===0);player.object.position.set(p.dx,p.dz,p.dy/Math.sin(35*Math.PI/180));player.object.updateMatrixWorld(true);
 const morphs:THREE.Mesh[]=[];const staticNodes:{object:THREE.Mesh;matrix:number[]}[]=[];player.content.traverse(o=>{const mesh=o as THREE.Mesh;if(!mesh.isMesh)return;if(mesh.morphTargetInfluences?.length)morphs.push(mesh);else staticNodes.push({object:mesh,matrix:mesh.matrix.toArray()})});check('Three crown primitives',morphs.length===3);check('Fixed wood exists',staticNodes.length>0);check('One reviewed clip',player.clipNames.length===1);player.select(player.clipNames[0]!,{mode:'loop',cycleTicks:56});
 try{
  for(let tick=0;tick<=112;tick++){
   player.seek(tick);const phase=Math.floor((tick%56)/4);for(const mesh of morphs){const w=mesh.morphTargetInfluences!;check('13 targets tick'+tick,w.length===13);check('Exact held pose tick'+tick,w.every((n,i)=>n===(phase===i+1?1:0)))}for(const n of staticNodes)check('Fixed wood transform tick'+tick,n.matrix.every((value,i)=>value===n.object.matrix.elements[i]));if(tick<=56&&tick%4===0)await hooks.phase?.(tick,player.object);
  }
  player.seek(28);player.pause();player.advance(1);check('Paused cursor unchanged',player.tick===28);player.play();player.advance(4/25);check('Playback advances four native ticks',player.tick===32);player.pause();player.seek(56);check('Loop closes to basis',player.tick===0&&morphs.every(m=>m.morphTargetInfluences!.every(x=>x===0)));
 }finally{player.dispose();loader.dispose()}
 let rejected=false;try{player.seek(0)}catch{rejected=true}check('Disposed player rejects seek',rejected);
 return{status:'PASS_TREE42_REAL_LOADER_MORPH_CYCLE',checks,model_sha256:digest,source_clock_hz:25,cycle_ticks:56,canonical_writes:false,scope:'Saved approved physical crown only; no mission activation, static replacement, native artwork parity or other crown propagation.'};
}
