/** Private integration adapter; callers provide reviewed partitioned receivers and endpoints. */
import * as THREE from '../../app/node_modules/three/build/three.module.js';
import {PatchDisplay} from '../../app/src/patch-display.ts';
export interface ApertureBinding {
  patch: string;
  mission: string;
  endpointParent: THREE.Object3D;
}
/** Existing reveal rules own the caps; a containing scene explicitly leases receiver visibility. */
export class PrivateApertureSession {
  private readonly display = new PatchDisplay();
  private readonly initialVisibility: boolean[];
  private mission: string | undefined;
  private physical = false;
  private disposed = false;
  private readonly privateRoot: THREE.Object3D;
  private readonly originalReceivers: readonly THREE.Object3D[];
  private readonly bindings: readonly ApertureBinding[];
  constructor(privateRoot: THREE.Object3D, originalReceivers: readonly THREE.Object3D[], bindings: readonly ApertureBinding[]) {
    this.privateRoot=privateRoot;this.originalReceivers=originalReceivers;this.bindings=bindings;
    if (!bindings.length || new Set(bindings.map(b=>b.patch)).size!==bindings.length)
      throw new Error('Unique nonempty patch bindings required');
    if(new Set(originalReceivers).size!==originalReceivers.length)
      throw new Error('Duplicate original receiver lease');
    this.initialVisibility=originalReceivers.map(o=>o.visible);
    this.privateRoot.visible=false;
    this.refresh();
  }
  private live(){if(this.disposed)throw new Error('Aperture session is disposed');}
  private refresh(){
    this.display.apply(this.privateRoot);
    for(const parent of new Set(this.bindings.map(b=>b.endpointParent)))
      parent.visible=this.bindings.some(b=>b.endpointParent===parent&&b.mission===this.mission);
    this.privateRoot.visible=this.physical;
    this.originalReceivers.forEach((receiver,i)=>receiver.visible=this.physical?false:this.initialVisibility[i]!);
  }
  selectMission(mission:string){
    this.live();if(!this.bindings.some(b=>b.mission===mission))throw new Error('Unknown aperture mission');
    this.display.clear();this.mission=mission;this.refresh();
  }
  selectPhysical(physical:boolean){
    this.live();if(physical&&!this.mission)throw new Error('Select a mission before physical aperture mode');
    this.physical=physical;this.refresh();
  }
  setApplied(patch:string,applied:boolean){
    this.live();if(!this.bindings.some(b=>b.patch===patch&&b.mission===this.mission))
      throw new Error('Patch is outside the selected aperture mission');
    this.display.set(patch,applied);this.refresh();
  }
  reset(){this.live();this.display.clear();this.refresh();}
  dispose(){
    if(this.disposed)return;
    this.display.clear();this.physical=false;this.mission=undefined;this.refresh();this.disposed=true;
  }
}
