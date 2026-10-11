/** Private real-loader endpoint lifecycle proof; no canonical publication. */
import { StateDelivery } from '../../app/src/state-delivery.ts';
import { SceneAssetLoader } from '../../app/src/scene-assets.ts';
import { TextureDisplay } from '../../app/src/texture-display.ts';
import type { MissionStateSource } from '../../app/src/mission-state-layer.ts';
import type { StateDeliveryContract } from '../../shared/src/state-delivery.ts';

type Pin = {path:string;sha256:string};
type Row = {id:string;mission:string;site:string;terminal_tick:number;contract:Pin};
type Config = {bindings:Row[];asset_handle:{mapping:Record<string,string>};native_reader:{mapping:{logical_path:string;file:string;sha256:string}[]}};
type Request = {reader_config:Pin;sources:Record<string,{data:Pin;level:Pin}>};
type Phase = 'initial'|'applied'|'reset'|'native-restored'|'mission-retired'|'disposed';
type Hooks = {phase?:(row:Row,phase:Phase,delivery:StateDelivery)=>Promise<void>|void;maxAnisotropy?:number};
function check(value:unknown,label:string):asserts value {if(!value)throw Error(label)}
async function digest(bytes:Uint8Array){return [...new Uint8Array(await crypto.subtle.digest('SHA-256',new Uint8Array(bytes)))].map(v=>v.toString(16).padStart(2,'0')).join('')}
export async function runMoundLifecycle(request:Request,hooks:Hooks={}){
  async function bytes(pin:Pin){const response=await fetch('/@fs'+pin.path);check(response.ok,'HTTP '+pin.path);const raw=new Uint8Array(await response.arrayBuffer());check(await digest(raw)===pin.sha256,'SHA '+pin.path);return raw;}
  async function json<T>(pin:Pin):Promise<T>{return JSON.parse(new TextDecoder().decode(await bytes(pin)))}
  const config=await json<Config>(request.reader_config);check(config.bindings.length===32,'Expected32 controls');check(new Set(config.bindings.map(r=>r.site)).size===20,'Expected20 sites');
  const contracts=new Map<string,StateDeliveryContract>(),modelPins=new Map<string,Pin>();
  for(const row of config.bindings){const contract=await json<StateDeliveryContract>(row.contract);contracts.set(row.id,contract);for(const family of contract.families)for(const endpoint of [family.physical.initial,family.physical.applied])if(Array.isArray(endpoint))for(const source of endpoint){check(source.model&&source.model_sha256,'Model pin missing');const path=config.asset_handle.mapping[source.model];check(path,'Model route missing');const prior=modelPins.get(source.model);check(!prior||prior.sha256===source.model_sha256,'Model pin collision');modelPins.set(source.model,{path,sha256:source.model_sha256});}}
  const nativePins=new Map(config.native_reader.mapping.map(p=>[p.logical_path,{path:p.file,sha256:p.sha256}]));
  // Read-only handles feed the actual production SceneAssetLoader; no scene substitutes.
  function directory(prefix=''):FileSystemDirectoryHandle{return {kind:'directory',name:prefix,async getDirectoryHandle(name:string){check(!name.includes('/')&&name!=='.'&&name!=='..','Unsafe directory');return directory(prefix+name+'/')},async getFileHandle(name:string){check(!name.includes('/')&&name!=='.'&&name!=='..','Unsafe filename');const logical=prefix+name;return {kind:'file',name,async getFile(){const pin=modelPins.get(logical);if(!pin)throw new DOMException('Unmapped '+logical,'NotFoundError');return new File([new Uint8Array(await bytes(pin))],name)}}}} as unknown as FileSystemDirectoryHandle;}
  const library=directory(),display=new TextureDisplay();let loads=0,disposals=0;
  const delivery=new StateDelivery(root=>{const loader=new SceneAssetLoader(root);return {async load(source){const scene=await loader.load(source);display.apply(scene,hooks.maxAnisotropy??1);loads++;return scene},dispose(){disposals++;loader.dispose()}}});
  const sourceCache=new Map<string,MissionStateSource>(),results:unknown[]=[];
  async function source(row:Row,contract:StateDeliveryContract){if(!sourceCache.has(row.mission)){const pins=request.sources[row.mission];check(pins,'Missing mission source');sourceCache.set(row.mission,{name:row.mission,data:await json(pins.data),level:await json(pins.level),camera:{kind:'oblique-orthographic',elevation_deg:contract.native.camera_elevation_deg}})}return sourceCache.get(row.mission)!;}
  const nativeRead=async(resource:Pin)=>{const pin=nativePins.get(resource.path);check(pin,'Unmapped native resource '+resource.path);check(pin.sha256===resource.sha256,'Native resource pin conflict');return bytes(pin)};
  let previous:Row|undefined,previousFamily:string|undefined;
  try{
    for(const row of config.bindings){
      const contract=contracts.get(row.id)!;check(contract.families.length===1,'Single focused family required');const family=contract.families[0]!.id;const priorRoots=[...delivery.physical.children];
      check(await delivery.set(contract,await source(row,contract),library,nativeRead),'Delivery load failed');
      check(delivery.native.mission===row.mission,'Mission source did not change');check(delivery.physical.children.length===2,'Endpoint pair missing');
      if(previous){check(priorRoots.every(root=>root.parent===null),'Prior endpoint still attached');check(!delivery.hasFamily(previousFamily!),'Previous family retained');check(disposals===results.length,'Retired loader not disposed');await hooks.phase?.(previous,'mission-retired',delivery);}
      const initialHash=await digest(delivery.native.pixels().data);
      const [initial,applied]=delivery.physical.children;
      check(initial!.children.length===1,'Initial endpoint absent');const absent=row.id==='mission-Tac19_FoB_EC-patch-015';check(applied!.children.length===(absent?0:1),'Incorrect applied presence');
      delivery.selectMode('physical-endpoint');delivery.selectEndpoint(family,'initial');check(delivery.physical.visible&&initial!.visible&&!applied!.visible,'Initial selection');await hooks.phase?.(row,'initial',delivery);
      delivery.selectEndpoint(family,'applied');check(!initial!.visible&&applied!.visible,'Applied selection');await hooks.phase?.(row,'applied',delivery);
      let rejected=false;try{delivery.setPlaying(true)}catch{rejected=true}check(rejected,'Physical endpoint playback allowed');
      delivery.selectMode('native-art');delivery.seekFamily(family,row.terminal_tick);const terminalHash=await digest(delivery.native.pixels().data);check(delivery.familyTick(family)===row.terminal_tick,'Terminal tick mismatch');
      delivery.selectMode('physical-endpoint');delivery.selectEndpoint(family,'applied');delivery.selectMode('native-art');check(!delivery.physical.visible,'Native mode still shows physical');check(await digest(delivery.native.pixels().data)===terminalHash,'Native pixels changed on mode restore');await hooks.phase?.(row,'native-restored',delivery);
      delivery.reset(family);delivery.selectEndpoint(family,'initial');check(delivery.familyTick(family)===undefined,'Reset clock retained');check(await digest(delivery.native.pixels().data)===initialHash,'Reset did not restore native pixels');delivery.selectMode('physical-endpoint');check(initial!.visible&&!applied!.visible,'Physical reset selection');await hooks.phase?.(row,'reset',delivery);
      results.push({id:row.id,mission:row.mission,site:row.site,initial_rgba_sha256:initialHash,terminal_rgba_sha256:terminalHash,applied_absent:absent,actual_loader:true,initial_applied_reset:true,native_mode_restore:true});previous=row;previousFamily=family;
    }
  }finally{delivery.dispose()}
  check(!delivery.ready&&!delivery.native.ready&&delivery.physical.children.length===0&&!delivery.physical.parent,'Disposal retained state');check(disposals===32,'Loader disposal count');check(loads===63,'Expected32 initial and31 applied loads');await hooks.phase?.(previous!,'disposed',delivery);
  return {status:'PASS_BROWSER_REAL_MOUND_DELIVERY_LIFECYCLE',controls:results,sites:20,loads,disposals,canonical_writes:false,scope:'Production StateDelivery and SceneAssetLoader; controlled endpoint/native restoration and mission retirement. Not full editor UI or gameplay.'};
}
