import { StateEventClocks, type StateClockBatch } from "../../shared/src/state-event-clocks.ts";
import { ReviewedNodePoses } from "./reviewed-node-poses.ts";
import * as THREE from "three";
import {
  validateStateDelivery,
  physicalEndpointSources,
  physicalFamilyEndpoints,
  physicalPatchPose,
  type StateDeliveryContract,
} from "../../shared/src/state-delivery.ts";
import { NativeStatePresentation, type NativeResourceReader } from "./native-state-presentation.ts";
import type { MissionStateSource } from "./mission-state-layer.ts";
import { SceneAssetLoader, captureLoadedStateAppearance } from "./scene-assets.ts";
import { disposeObjectResources } from "./resources.ts";

export type StateDeliveryMode = "native-art" | "physical-endpoint" | "physical-motion";
type FamilyRoots = { initial: THREE.Group; applied: THREE.Group; frames: THREE.Group[] };
type Family = StateDeliveryContract["families"][number];
/** One containing renderer owns the clock. Only explicitly reviewed patch poses support physical motion. */
export class StateDelivery {
  readonly native = new NativeStatePresentation();
  readonly physical = new THREE.Group();
  private contract: StateDeliveryContract | undefined;
  private roots = new Map<string, FamilyRoots>();
  private starts = new Map<string, number>();
  private sourceClocks: StateEventClocks | undefined;
  private nodePoses = new Map<string, ReviewedNodePoses>();
  private selectedFamily: string | undefined;
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
  hasFamily(id: string) {
    return !!this.contract?.families.some((family) => family.id === id);
  }
  get mode() {
    return this.currentMode;
  }
  clear() {
    this.epoch++;
    this.native.clear();
    this.physical.clear();
    this.physical.visible = false;
    for (const poses of this.nodePoses.values()) poses.reset();
    this.nodePoses.clear();
    this.roots.clear();
    this.starts.clear();
    this.sourceClocks = undefined;
    this.selectedFamily = undefined;
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
      const roots = new Map<string, FamilyRoots>();
      for (const family of frozen.families) {
        const endpoints = physicalFamilyEndpoints(family);
        const groups = endpoints.map(() => new THREE.Group());
        const pair = { initial: groups[0]!, applied: groups[1]!, frames: groups.slice(2) };
        for (const [index, endpoint] of endpoints.entries()) {
          const state = index === 0 ? "initial" : index === 1 ? "applied" : `frame-${index - 2}`;
          for (const binding of physicalEndpointSources(endpoint)) {
            const asset = await loader.load(binding);
            owned.add(asset);
            if (!current()) return false;
            if (captureLoadedStateAppearance(asset))
              throw new Error(
                "Physical endpoint contains animation; endpoint-only contract required",
              );
            const clone = asset.clone(true);
            if (binding.position) clone.position.add(new THREE.Vector3(...binding.position));
            clone.userData = {
              ...clone.userData,
              family: family.id,
              endpoint: state,
              representation: "physical-endpoint",
            };
            groups[index]!.add(clone);
          }
          groups[index]!.visible = false;
        }
        if (
          family.physical.motion?.kind === "native-patch-node-poses" ||
          family.physical.motion?.kind === "native-element-node-poses"
        )
          this.nodePoses.set(
            family.id,
            new ReviewedNodePoses(pair.applied, family.physical.motion.node_frames),
          );
        if (family.physical.motion?.kind === "native-element-stage-poses")
          this.nodePoses.set(
            family.id,
            new ReviewedNodePoses(
              pair.applied,
              family.physical.motion.stages.flatMap((stage) =>
                stage.node_frames.filter((frame) => frame !== null),
              ),
            ),
          );
        roots.set(family.id, pair);
      }
      if (!current()) return false;
      this.roots = roots;
      this.contract = frozen;
      for (const pair of roots.values())
        this.physical.add(pair.initial, pair.applied, ...pair.frames);
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
    if (!this.ready || !["native-art", "physical-endpoint", "physical-motion"].includes(mode))
      throw new Error("State delivery mode is unavailable");
    if (mode === "physical-motion" && !this.family(this.selectedFamily!).physical.motion)
      throw new Error("Selected family has no reviewed physical motion");
    const previousPose = this.motionEndpoint;
    this.currentMode = mode;
    this.physical.visible = mode !== "native-art";
    if (mode === "physical-motion") this.sampleMotion();
    else if (previousPose && this.selectedFamily)
      this.selectEndpoint(this.selectedFamily, previousPose);
    if (mode === "physical-endpoint") this.native.setPlaying(false);
  }
  selectEndpoint(id: string, endpoint: "initial" | "applied") {
    const family = this.family(id);
    if (this.mode === "physical-motion" && !family.physical.motion)
      throw new Error("Selected family has no reviewed physical motion");
    if (!["initial", "applied"].includes(endpoint)) throw new Error("Unknown physical endpoint");
    this.selectedFamily = id;
    for (const poses of this.nodePoses.values()) poses.reset();
    for (const [key, pair] of this.roots) {
      pair.initial.position.set(0, 0, 0);
      pair.applied.position.set(0, 0, 0);
      pair.initial.visible = key === id && endpoint === "initial";
      pair.applied.visible = key === id && endpoint === "applied";
      for (const frame of pair.frames) frame.visible = false;
    }
    this.sampleMotion();
  }
  /** Apply independently observed owner/patch clocks without activating sibling state. */
  sampleSourceState(batch: StateClockBatch) {
    if (!this.contract) throw new Error("State delivery is not ready");
    if (!this.sourceClocks && this.starts.size)
      throw new Error("Reload preview state before binding source clocks");
    this.native.assertCanSampleStateClocks(batch);
    const clocks = this.sourceClocks ?? new StateEventClocks(this.contract);
    const samples = clocks.apply(batch);
    this.native.setPlaying(false);
    this.sourceClocks = clocks;
    for (const sample of samples) {
      if (sample.kind === "element") {
        this.native.setElementState(sample.id, sample.active, sample.tick);
        if (sample.visible !== undefined) this.native.setElementVisible(sample.id, sample.visible);
      } else if (sample.kind === "patch")
        this.native.setPatchState(sample.id, sample.phase, sample.tick);
      else this.native.setBackgroundState(sample.id, sample.phase, sample.tick);
    }
    this.sampleMotion();
  }
  seekFamily(id: string, tick: number) {
    if (this.sourceClocks)
      throw new Error("Source events own state clocks; reload to use preview playback");
    const family = this.family(id);
    if (!Number.isSafeInteger(tick) || tick < 0) throw new Error("Invalid state delivery tick");
    this.starts.set(id, this.native.tick - tick);
    for (const element of [...family.element_ids, ...(family.hidden_initial_element_ids ?? [])])
      this.native.setElementState(element, true, tick);
    for (const patch of family.patch_ids ?? []) this.native.setPatchState(patch, "forward", tick);
    for (const background of family.background_ids)
      this.native.setBackgroundState(background, "forward", tick);
    this.sampleMotion();
  }
  activate(id: string) {
    this.seekFamily(id, 0);
  }
  reset(id: string) {
    if (this.sourceClocks)
      throw new Error("Source events own state clocks; reset through source events");
    const family = this.family(id);
    this.starts.delete(id);
    for (const element of [...family.element_ids, ...(family.hidden_initial_element_ids ?? [])])
      this.native.setElementState(element, false);
    for (const patch of family.patch_ids ?? []) this.native.setPatchState(patch, "initial");
    for (const background of family.background_ids)
      this.native.setBackgroundState(background, "initial");
    this.sampleMotion();
  }
  familyTick(id: string): number | undefined {
    this.family(id);
    if (this.sourceClocks) return this.sourceClocks.physicalTick(id);
    const start = this.starts.get(id);
    return start === undefined ? undefined : this.native.tick - start;
  }
  setPlaying(playing: boolean) {
    if (this.sourceClocks && playing) throw new Error("Source events own state playback");
    if (!this.ready) throw new Error("State delivery is not ready");
    if (playing && this.mode === "physical-endpoint")
      throw new Error("Physical endpoints have no transition playback");
    this.native.setPlaying(playing);
  }
  private sampleMotion() {
    if (this.mode !== "physical-motion" || !this.selectedFamily) return;
    const pose =
      this.sourceClocks?.physicalPose(this.selectedFamily) ??
      physicalPatchPose(this.contract!, this.selectedFamily, this.familyTick(this.selectedFamily));
    const pair = this.roots.get(this.selectedFamily)!;
    const nodes = this.nodePoses.get(this.selectedFamily);
    if (pose.node_frame === undefined) nodes?.reset();
    else {
      if (!nodes) throw new Error("Reviewed node pose binding missing");
      nodes.apply(pose.node_frame);
    }
    pair.initial.position.fromArray(pose.translation);
    pair.applied.position.set(0, 0, 0);
    if (pose.model_frame !== undefined && !pair.frames[pose.model_frame])
      throw new Error("Reviewed model frame binding missing");
    pair.initial.visible = pose.model_frame === undefined && pose.endpoint === "initial";
    pair.applied.visible = pose.model_frame === undefined && pose.endpoint === "applied";
    pair.frames.forEach((frame, index) => {
      frame.visible = index === pose.model_frame;
    });
    if (
      pose.visible === false ||
      (this.sourceClocks && !this.sourceClocks.physicalVisible(this.selectedFamily))
    ) {
      pair.initial.visible = false;
      pair.applied.visible = false;
      for (const frame of pair.frames) frame.visible = false;
    }
  }
  get motionEndpoint(): "initial" | "applied" | undefined {
    if (this.mode !== "physical-motion" || !this.selectedFamily) return undefined;
    return (
      this.sourceClocks?.physicalPose(this.selectedFamily) ??
      physicalPatchPose(this.contract!, this.selectedFamily, this.familyTick(this.selectedFamily))
    ).endpoint;
  }
  advance(seconds: number) {
    if (this.sourceClocks) return false;
    const changed = this.native.advance(seconds);
    if (changed) this.sampleMotion();
    return changed;
  }
  dispose() {
    if (this.disposed) return;
    this.disposed = true;
    this.clear();
    this.native.dispose();
    this.physical.removeFromParent();
  }
}
