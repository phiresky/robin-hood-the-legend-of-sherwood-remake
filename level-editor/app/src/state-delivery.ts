import * as THREE from "three";
import {
  validateStateDelivery,
  type StateDeliveryContract,
} from "../../shared/src/state-delivery.ts";
import { NativeStatePresentation, type NativeResourceReader } from "./native-state-presentation.ts";
import type { MissionStateSource } from "./mission-state-layer.ts";
import { SceneAssetLoader, captureLoadedStateAppearance } from "./scene-assets.ts";
import { disposeObjectResources } from "./resources.ts";

export type StateDeliveryMode = "native-art" | "physical-endpoint";
type Family = StateDeliveryContract["families"][number];
/** One containing renderer owns the clock. Physical endpoints never interpolate into motion. */
export class StateDelivery {
  readonly native = new NativeStatePresentation();
  readonly physical = new THREE.Group();
  private contract: StateDeliveryContract | undefined;
  private roots = new Map<string, { initial: THREE.Group; applied: THREE.Group }>();
  private starts = new Map<string, number>();
  private epoch = 0;
  private disposed = false;
  private cleanup: (() => void) | undefined;
  private currentMode: StateDeliveryMode = "native-art";
  private readonly loader: (
    root: FileSystemDirectoryHandle,
  ) => Pick<SceneAssetLoader, "load" | "dispose">;
  constructor(
    loader: (root: FileSystemDirectoryHandle) => Pick<SceneAssetLoader, "load" | "dispose"> = (
      root,
    ) => new SceneAssetLoader(root),
  ) {
    this.loader = loader;
    this.physical.visible = false;
  }
  get ready() {
    return !!this.contract;
  }
  get mode() {
    return this.currentMode;
  }
  clear() {
    this.epoch++;
    this.native.clear();
    this.physical.clear();
    this.physical.visible = false;
    this.roots.clear();
    this.starts.clear();
    this.contract = undefined;
    this.currentMode = "native-art";
    this.cleanup?.();
    this.cleanup = undefined;
  }
  async set(
    contract: StateDeliveryContract,
    source: MissionStateSource,
    library: FileSystemDirectoryHandle,
    read: NativeResourceReader,
  ) {
    if (this.disposed) throw new Error("State delivery is disposed");
    this.clear();
    const epoch = this.epoch,
      frozen = structuredClone(contract);
    validateStateDelivery(frozen);
    const current = () => !this.disposed && this.epoch === epoch;
    if (!(await this.native.set(frozen.native, source, read)) || !current()) return false;
    let loader: Pick<SceneAssetLoader, "load" | "dispose">;
    try {
      loader = this.loader(library);
    } catch (error) {
      if (!current()) return false;
      this.clear();
      throw error;
    }
    const owned = new Set<THREE.Object3D>();
    let retired = false,
      pending = true,
      released = false;
    const release = () => {
      if (retired && !pending && !released) {
        released = true;
        disposeObjectResources([...owned]);
        loader.dispose();
      }
    };
    this.cleanup = () => {
      retired = true;
      release();
    };
    try {
      const roots = new Map<string, { initial: THREE.Group; applied: THREE.Group }>();
      for (const family of frozen.families) {
        const pair = { initial: new THREE.Group(), applied: new THREE.Group() };
        for (const state of ["initial", "applied"] as const) {
          for (const binding of family.physical[state]) {
            const asset = await loader.load(binding);
            owned.add(asset);
            if (!current()) return false;
            if (captureLoadedStateAppearance(asset))
              throw new Error(
                "Physical endpoint contains animation; endpoint-only contract required",
              );
            const clone = asset.clone(true);
            clone.userData = {
              ...clone.userData,
              family: family.id,
              endpoint: state,
              representation: "physical-endpoint",
            };
            pair[state].add(clone);
          }
          pair[state].visible = false;
        }
        roots.set(family.id, pair);
      }
      if (!current()) return false;
      this.roots = roots;
      this.contract = frozen;
      for (const pair of roots.values()) this.physical.add(pair.initial, pair.applied);
      this.selectEndpoint(frozen.families[0]!.id, "initial");
      return true;
    } catch (error) {
      if (!current()) return false;
      this.clear();
      throw error;
    } finally {
      pending = false;
      release();
    }
  }
  private family(id: string): Family {
    const family = this.contract?.families.find((f) => f.id === id);
    if (!family) throw new Error(`State delivery missing family: ${id}`);
    return family;
  }
  selectMode(mode: StateDeliveryMode) {
    if (!this.ready || !["native-art", "physical-endpoint"].includes(mode))
      throw new Error("State delivery mode is unavailable");
    this.currentMode = mode;
    this.physical.visible = mode === "physical-endpoint";
    if (mode === "physical-endpoint") this.native.setPlaying(false);
  }
  selectEndpoint(id: string, endpoint: "initial" | "applied") {
    this.family(id);
    if (!["initial", "applied"].includes(endpoint)) throw new Error("Unknown physical endpoint");
    for (const [key, pair] of this.roots) {
      pair.initial.visible = key === id && endpoint === "initial";
      pair.applied.visible = key === id && endpoint === "applied";
    }
  }
  seekFamily(id: string, tick: number) {
    const family = this.family(id);
    if (!Number.isSafeInteger(tick) || tick < 0) throw new Error("Invalid state delivery tick");
    this.starts.set(id, this.native.tick - tick);
    for (const element of family.element_ids) this.native.setElementState(element, true, tick);
    for (const background of family.background_ids)
      this.native.setBackgroundState(background, "forward", tick);
  }
  activate(id: string) {
    this.seekFamily(id, 0);
  }
  reset(id: string) {
    const family = this.family(id);
    this.starts.delete(id);
    for (const element of family.element_ids) this.native.setElementState(element, false);
    for (const background of family.background_ids)
      this.native.setBackgroundState(background, "initial");
  }
  familyTick(id: string): number | undefined {
    this.family(id);
    const start = this.starts.get(id);
    return start === undefined ? undefined : this.native.tick - start;
  }
  setPlaying(playing: boolean) {
    if (!this.ready) throw new Error("State delivery is not ready");
    if (playing && this.mode !== "native-art")
      throw new Error("Physical endpoints have no transition playback");
    this.native.setPlaying(playing);
  }
  advance(seconds: number) {
    return this.native.advance(seconds);
  }
  dispose() {
    if (this.disposed) return;
    this.disposed = true;
    this.clear();
    this.native.dispose();
    this.physical.removeFromParent();
  }
}
