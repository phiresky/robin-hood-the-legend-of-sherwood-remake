/** Private current-receiver preparation; byte-checked scenes are supplied by the production loader. */
import * as THREE from '../../app/node_modules/three/build/three.module.js';
import type {PreparedStateAperturePreview} from '../../app/src/state-aperture-preview.ts';
interface Pin {path:string;sha256:string}
interface ModelScenes {sha256:string;scenes:THREE.Group[]}
interface Integration {
  receivers:{asset:string;model:Pin;partition_scene:string}[];
  endpoints:Record<'initial'|'applied',{model:Pin;scene:string}>;
  sites:{id:string;patches:string[];endpoint_position:[number,number,number];receiver_asset:string}[];
  controls:{id:string;mission:string;family:string;patches:string[];site:string}[];
}
function namedScene(model:ModelScenes,pin:Pin,name:string){
  if(model.sha256!==pin.sha256)throw new Error('Aperture model hash differs from the pinned integration');
  const matches=model.scenes.filter(s=>s.name===name);
  if(matches.length!==1)throw new Error('Missing or duplicate aperture scene '+name);
  return matches[0]!.clone(true);
}
export function prepareCurrentHoleApertures(
  integration:Integration,
  receivers:Map<string,{original:THREE.Object3D;model:ModelScenes}>,
  endpoints:Record<'initial'|'applied',ModelScenes>,
  focusPatch:string,
):PreparedStateAperturePreview{
  if(integration.sites.length!==17||integration.controls.length!==30||receivers.size!==2)
    throw new Error('Unexpected current hole integration scope');
  const root=new THREE.Group();root.name='Private current hole aperture staging';root.rotation.x=Math.PI/2;
  const originals:THREE.Object3D[]=[];
  for(const receiver of integration.receivers){
    const loaded=receivers.get(receiver.asset);if(!loaded)throw new Error('Missing current receiver '+receiver.asset);
    originals.push(loaded.original);root.add(namedScene(loaded.model,receiver.model,receiver.partition_scene));
  }
  const focus=integration.controls.find(c=>c.id===focusPatch);if(!focus)throw new Error('Unknown focused hole control');
  const sites=new Map<string,THREE.Group>();
  for(const site of integration.sites){
    if(sites.has(site.id))throw new Error('Duplicate aperture site');
    const parent=new THREE.Group();parent.name=site.id;parent.userData.aperture_site=site.id;parent.visible=integration.controls.some(c=>c.site===site.id&&c.mission===focus.mission);root.add(parent);sites.set(site.id,parent);
    for(const phase of ['initial','applied']as const){
      const model=integration.endpoints[phase];const scene=namedScene(endpoints[phase],model.model,model.scene);scene.name=site.id+'/'+phase;scene.position.add(new THREE.Vector3(...site.endpoint_position));scene.userData[phase==='initial'?'reveal_hide_when_applied':'reveal_show_when_applied']=[...site.patches];parent.add(scene);
    }
    const caps:THREE.Object3D[]=[];root.traverse(o=>{if(o.userData.aperture_component===site.id)caps.push(o)});
    if(!caps.length||caps.some(o=>JSON.stringify(o.userData.reveal_hide_when_applied)!==JSON.stringify(site.patches)))throw new Error('Missing or mismatched receiver cap '+site.id);
  }
  const bindings=integration.controls.filter(c=>c.id===focusPatch).map(control=>{
    const parent=sites.get(control.site),site=integration.sites.find(s=>s.id===control.site);
    if(!parent||!site?.patches.includes(control.id)||control.patches.length!==1||control.patches[0]!==control.id)throw new Error('Wrong source control/site binding');
    return{family:control.family,mission:control.mission,patches:[...control.patches],endpointParent:parent};
  });
  root.updateMatrixWorld(true);return{root,originalReceivers:originals,bindings};
}
