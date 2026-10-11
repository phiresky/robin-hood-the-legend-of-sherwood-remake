import * as THREE from 'three';
import { safeLibraryPath, type Level3D, type SceneAssetSource } from '@rle/shared';
import { SceneAssetLoader } from './scene-assets.ts';
import { disposeObjectResources } from './resources.ts';
import type { PreparedStateAperturePreview } from './state-aperture-preview.ts';

type Transform = {dx:number;dy:number;dz:number;rot_deg:number};
export interface StateApertureManifest {
  version:1;
  map:string;
  receivers:{asset:string;group?:string;model_sha256:string;descriptor_sha256:string;placement:Transform;parts:string[];replacement:SceneAssetSource}[];
  endpoints:Record<'initial'|'applied',SceneAssetSource>;
  sites:{id:string;patches:string[];endpoint_position:[number,number,number]}[];
  controls:{id:string;mission:string;family:string;patches:string[];site:string}[];
}
export function validateStateApertureManifest(value:unknown):asserts value is StateApertureManifest {
  const data=value as StateApertureManifest,hash=(s:unknown)=>typeof s==='string'&&/^[a-f0-9]{64}$/.test(s);
  const source=(s:SceneAssetSource)=>s&&typeof s.id==='string'&&safeLibraryPath(s.model)&&hash(s.model_sha256)&&typeof s.model_scene==='string'&&Array.isArray(s.resources)&&s.resources.length===0;
  if(!data||data.version!==1||typeof data.map!=='string'||!Array.isArray(data.receivers)||!data.receivers.length||!Array.isArray(data.sites)||!data.sites.length||!Array.isArray(data.controls)||!data.controls.length||!source(data.endpoints?.initial)||!source(data.endpoints?.applied))throw Error('Invalid aperture manifest');
  const receivers=new Set<string>(),sites=new Set<string>(),controls=new Set<string>();
  for(const r of data.receivers){if(!r.asset||receivers.has(r.asset)||!hash(r.model_sha256)||!hash(r.descriptor_sha256)||!source(r.replacement)||!r.placement||!['dx','dy','dz','rot_deg'].every(k=>Number.isFinite(r.placement[k as keyof Transform]))||!Array.isArray(r.parts)||r.parts.some(p=>typeof p!=='string')||(r.group!==undefined&&typeof r.group!=='string'))throw Error('Invalid aperture receiver');receivers.add(r.asset)}
  for(const s of data.sites){if(!s.id||sites.has(s.id)||!Array.isArray(s.patches)||!s.patches.length||s.patches.some(p=>typeof p!=='string'||!p)||!Array.isArray(s.endpoint_position)||s.endpoint_position.length!==3||!s.endpoint_position.every(Number.isFinite))throw Error('Invalid aperture site');sites.add(s.id)}
  for(const c of data.controls){if(!c.id||controls.has(c.id)||!c.family||!c.mission||!sites.has(c.site)||!Array.isArray(c.patches)||c.patches.length!==1||c.patches[0]!==c.id||!data.sites.find(s=>s.id===c.site)!.patches.includes(c.id))throw Error('Invalid aperture control');controls.add(c.id)}
}
export function verifyStateApertureReceivers(manifest:StateApertureManifest,document:Level3D){
  validateStateApertureManifest(manifest);
  if(document.map!==manifest.map)throw Error('Aperture map changed');
  for(const receiver of manifest.receivers){
    const sources=[...document.sceneAssets,...document.assetSources??[]].filter(s=>s.id===receiver.asset);
    if(sources.length!==1||sources[0]!.model_sha256!==receiver.model_sha256||sources[0]!.descriptor_sha256!==receiver.descriptor_sha256)throw Error('Aperture receiver source changed: '+receiver.asset);
    if(receiver.group){
      const groups=document.groups.filter(g=>g.id===receiver.group),group=groups[0];
      if(groups.length!==1||group!.hidden||!Object.keys(receiver.placement).every(k=>group!.transform[k as keyof Transform]===receiver.placement[k as keyof Transform]))throw Error('Aperture receiver placement changed');
      const parts=document.objects.filter(o=>o.group===receiver.group);
      if(parts.length!==receiver.parts.length||parts.some(p=>p.hidden||!receiver.parts.includes(p.node)||Object.values(p.transform).some(v=>v!==0)))throw Error('Aperture receiver parts changed');
    }
  }
}
export async function prepareStateAperture(
  manifest:StateApertureManifest,document:Level3D,library:FileSystemDirectoryHandle,
  originals:ReadonlyMap<string,THREE.Object3D>,mission:string,family:string,
  loaderFactory:(library:FileSystemDirectoryHandle)=>Pick<SceneAssetLoader,'load'|'dispose'>=root=>new SceneAssetLoader(root),
):Promise<PreparedStateAperturePreview>{
  verifyStateApertureReceivers(manifest,document);
  const focused=manifest.controls.filter(c=>c.mission===mission&&c.family===family);
  if(focused.length!==1)throw Error('Missing focused aperture control');
  const root=new THREE.Group();root.name='Reviewed state apertures';root.rotation.x=Math.PI/2;
  const loader=loaderFactory(library),owned:THREE.Object3D[]=[];
  try{
    const originalReceivers:THREE.Object3D[]=[];
    for(const receiver of manifest.receivers){const original=originals.get(receiver.asset);if(!original)throw Error('Missing displayed aperture receiver');originalReceivers.push(original);const model=await loader.load(receiver.replacement);owned.push(model);root.add(model)}
    const endpoints={}as Record<'initial'|'applied',THREE.Group>;
    for(const phase of ['initial','applied']as const){endpoints[phase]=await loader.load(manifest.endpoints[phase]);owned.push(endpoints[phase])}
    let endpointParent:THREE.Group|undefined;
    for(const site of manifest.sites){
      const parent=new THREE.Group();parent.name=site.id;parent.visible=manifest.controls.some(c=>c.mission===mission&&c.site===site.id);root.add(parent);
      for(const phase of ['initial','applied']as const){const node=endpoints[phase].clone(true);node.position.add(new THREE.Vector3(...site.endpoint_position));node.userData[phase==='initial'?'reveal_hide_when_applied':'reveal_show_when_applied']=[...site.patches];parent.add(node)}
      const caps:THREE.Object3D[]=[];root.traverse(node=>{if(node.userData.aperture_component===site.id)caps.push(node)});
      if(!caps.length||caps.some(cap=>JSON.stringify(cap.userData.reveal_hide_when_applied)!==JSON.stringify(site.patches)))throw Error('Aperture cap binding changed');
      if(site.id===focused[0]!.site)endpointParent=parent;
    }
    if(!endpointParent)throw Error('Missing focused aperture site');
    return {root,originalReceivers,bindings:[{family,mission,patches:[...focused[0]!.patches],endpointParent}]};
  }catch(error){disposeObjectResources([...owned,root]);throw error}finally{loader.dispose()}
}
