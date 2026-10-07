import * as THREE from "three";
import { PatchDisplay } from "./patch-display.ts";
export interface StateApertureBinding {
  family: string;
  mission: string;
  patches: string[];
  endpointParent: THREE.Object3D;
}
export interface PreparedStateAperturePreview {
  root: THREE.Object3D;
  originalReceivers: THREE.Object3D[];
  bindings: StateApertureBinding[];
}
/** Reviewed cap fragments and endpoints share the existing reveal rules. */
export class StateAperturePreview {
  readonly root: THREE.Object3D;
  private readonly receivers: THREE.Object3D[];
  private readonly bindings: StateApertureBinding[];
  private readonly display = new PatchDisplay();
  private readonly originalVisibility: boolean[];
  private readonly originalMatrices: THREE.Matrix4[];
  private mission = "";
  private enabled = false;
  private disposed = false;
  constructor(prepared: PreparedStateAperturePreview) {
    this.root = prepared.root;
    this.receivers = [...prepared.originalReceivers];
    this.bindings = prepared.bindings.map((b) => ({ ...b, patches: [...b.patches] }));
    if (
      !this.receivers.length ||
      new Set(this.receivers).size !== this.receivers.length ||
      !this.bindings.length
    )
      throw new Error("Aperture preview requires unique original receivers and bindings");
    const keys = new Set<string>(),
      patches = new Set<string>();
    for (const b of this.bindings) {
      const key = JSON.stringify([b.mission, b.family]);
      if (!b.family || !b.mission || !b.patches.length || keys.has(key))
        throw new Error("Invalid aperture family binding");
      keys.add(key);
      for (const patch of b.patches) {
        if (!patch || patches.has(patch)) throw new Error("Duplicate aperture patch binding");
        patches.add(patch);
      }
      let parent: THREE.Object3D | null = b.endpointParent;
      while (parent && parent !== this.root) parent = parent.parent;
      if (!parent) throw new Error("Aperture endpoint is outside its prepared root");
    }
    for (const receiver of this.receivers) {
      let node: THREE.Object3D | null = this.root;
      while (node) {
        if (node === receiver) throw new Error("Aperture receiver contains its replacement");
        node = node.parent;
      }
      let ancestor: THREE.Object3D | null = receiver;
      while (ancestor) {
        if (ancestor === this.root) throw new Error("Original receiver is inside the replacement");
        ancestor = ancestor.parent;
      }
      receiver.updateWorldMatrix(true, false);
    }
    this.originalVisibility = this.receivers.map((r) => r.visible);
    this.originalMatrices = this.receivers.map((r) => r.matrixWorld.clone());
    this.root.visible = false;
    this.refresh();
  }
  private live() {
    if (this.disposed) throw new Error("Aperture preview is disposed");
  }
  private refresh() {
    if (this.enabled)
      for (let i = 0; i < this.receivers.length; i++) {
        this.receivers[i]!.updateWorldMatrix(true, false);
        if (!this.receivers[i]!.matrixWorld.equals(this.originalMatrices[i]!))
          throw new Error("Aperture receiver moved after preparation");
      }
    this.display.apply(this.root);
    for (const parent of new Set(this.bindings.map((b) => b.endpointParent)))
      parent.visible = this.bindings.some(
        (b) => b.endpointParent === parent && b.mission === this.mission,
      );
    this.root.visible = this.enabled;
    this.receivers.forEach(
      (r, i) => (r.visible = this.enabled ? false : this.originalVisibility[i]!),
    );
  }
  selectMission(mission: string) {
    this.live();
    if (!this.bindings.some((b) => b.mission === mission))
      throw new Error("Unknown aperture mission");
    this.display.clear();
    this.mission = mission;
    this.refresh();
  }
  handles(family: string) {
    return this.bindings.some((b) => b.mission === this.mission && b.family === family);
  }
  selectEnabled(enabled: boolean) {
    this.live();
    if (enabled && !this.mission) throw new Error("Aperture mission is not selected");
    this.enabled = enabled;
    this.refresh();
  }
  selectEndpoint(family: string, endpoint: "initial" | "applied") {
    this.live();
    const binding = this.bindings.find((b) => b.mission === this.mission && b.family === family);
    if (!binding || !["initial", "applied"].includes(endpoint))
      throw new Error("Unknown aperture endpoint");
    for (const patch of binding.patches) this.display.set(patch, endpoint === "applied");
    this.refresh();
  }
  dispose() {
    if (this.disposed) return;
    this.display.clear();
    this.enabled = false;
    this.mission = "";
    this.refresh();
    this.root.removeFromParent();
    this.disposed = true;
  }
}
