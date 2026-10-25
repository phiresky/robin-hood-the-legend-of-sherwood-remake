import { StateMaterialFrames } from "./state-material-frames.ts";
import { StateEventClocks, type StateClockBatch } from "../../shared/src/state-event-clocks.ts";
import { StateAppearancePlayer } from "./state-appearance-player.ts";
import { ComponentNodePoses } from "./component-node-poses.ts";
import { ReviewedNodePoses } from "./reviewed-node-poses.ts";
import * as THREE from "three";
import {
  validateStateDelivery,
  physicalEndpointSources,
  physicalFamilyEndpoints,
  physicalPatchPose,
  nativeRemainderFrame,
  nativeRemainderNodeFrame,
  nativeRemainderComponentFrames,
  type StateDeliveryContract,
} from "../../shared/src/state-delivery.ts";

/** A target clip's terminal pose must reproduce the pinned applied model mesh for mesh. */
export function assertClipTerminalMatchesApplied(applied: THREE.Object3D, clip: THREE.Object3D) {
  const meshes = (root: THREE.Object3D) => {
    root.updateMatrixWorld(true);
    const rows = new Map<string, THREE.Mesh>();
    root.traverse((node) => {
      if (!(node instanceof THREE.Mesh)) return;
      if (!node.name || rows.has(node.name))
        throw new Error("Target clip consistency requires uniquely named meshes");
      rows.set(node.name, node);
    });
    return rows;
  };
  const a = meshes(applied),
    b = meshes(clip);
  if (!a.size || a.size !== b.size || [...a.keys()].some((name) => !b.has(name)))
    throw new Error("Target clip meshes differ from the applied model");
  for (const [name, mesh] of a) {
    const other = b.get(name)!;
    const scale = Math.max(1, ...mesh.matrixWorld.elements.map(Math.abs));
    if (
      mesh.matrixWorld.elements.some(
        (v, i) => Math.abs(v - other.matrixWorld.elements[i]!) > 1e-5 * scale,
      )
    )
      throw new Error(`Target clip terminal pose differs from the applied model: ${name}`);
    const p = mesh.geometry.getAttribute("position"),
      q = other.geometry.getAttribute("position");
    if (!p || !q || p.count !== q.count || p.itemSize !== q.itemSize)
      throw new Error(`Target clip geometry differs from the applied model: ${name}`);
    for (let i = 0; i < p.count * p.itemSize; i++)
      if (p.array[i] !== q.array[i])
        throw new Error(`Target clip geometry differs from the applied model: ${name}`);
  }
}
import { NativeStatePresentation, type NativeResourceReader } from "./native-state-presentation.ts";
import type { MissionStateSource } from "./mission-state-layer.ts";
import { SceneAssetLoader, captureLoadedStateAppearance } from "./scene-assets.ts";
import { disposeObjectResources } from "./resources.ts";

export type StateDeliveryMode =
  | "native-art"
  | "native-remainder"
  | "physical-endpoint"
  | "physical-motion";
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
  private componentPoses = new Map<string, ComponentNodePoses>();
  private nodePoses = new Map<string, ReviewedNodePoses>();
  private clips = new Map<string, Partial<Record<"transition" | "final", StateAppearancePlayer>>>();
  private materialFrames = new Map<StateAppearancePlayer, StateMaterialFrames>();
  private selectedFamily: string | undefined;
  private selectedEndpoint: "initial" | "applied" = "initial";
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
    for (const poses of this.componentPoses.values()) poses.reset();
    for (const poses of this.nodePoses.values()) poses.reset();
    this.componentPoses.clear();
    this.nodePoses.clear();
    for (const frames of this.materialFrames.values()) frames.dispose();
    this.materialFrames.clear();
    for (const phases of this.clips.values())
      for (const player of Object.values(phases)) player.dispose();
    this.clips.clear();
    this.roots.clear();
    this.starts.clear();
    this.sourceClocks = undefined;
    this.selectedFamily = undefined;
    this.selectedEndpoint = "initial";
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
    try {
      const remainders = frozen.families.flatMap((family) => family.native_remainder ?? []);
      if (remainders.length && (!(await this.native.setRemainders(remainders, read)) || !current()))
        return false;
      for (const family of frozen.families) {
        const motion = family.physical.motion;
        if (motion?.kind === "native-patch-clips" && motion.transparent_initial_frames)
          this.native.assertTransparentPatchInitial(motion.patch_id);
      }
    } catch (error) {
      this.clear();
      throw error;
    }
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
            const motion = family.physical.motion;
            if (motion?.kind === "native-element-clip" && index === 2) {
              const asset = await loader.load(binding);
              owned.add(asset);
              if (!current()) return false;
              const appearance = captureLoadedStateAppearance(asset);
              if (!appearance) throw new Error("Target clip requires its pinned animated model");
              if (appearance.clips.length !== 1 || appearance.clips[0]!.name !== motion.clip)
                throw new Error("Target clip identity differs from model");
              // Keys end exactly at the source terminal tick: the clip is never stretched or cut.
              if (
                appearance.clips[0]!.tracks.some(
                  (track) => track.track.times.at(-1) !== family.body_terminal_tick,
                )
              )
                throw new Error("Target clip keys differ from the source terminal tick");
              const player = new StateAppearancePlayer(appearance, true);
              this.clips.set(family.id, { transition: player });
              player.select(motion.clip, {
                mode: "clamp",
                terminalTick: family.body_terminal_tick,
              });
              const clone = player.object;
              if (binding.position) clone.position.add(new THREE.Vector3(...binding.position));
              clone.userData = {
                ...clone.userData,
                family: family.id,
                endpoint: state,
                representation: "physical-endpoint",
              };
              groups[index]!.add(clone);
              player.sampleExternalTick(family.body_terminal_tick);
              assertClipTerminalMatchesApplied(groups[1]!, groups[index]!);
              player.sampleExternalTick(0);
              continue;
            }
            const phase =
              motion?.kind === "native-patch-clips"
                ? index === 2
                  ? "transition"
                  : index === 1 && !("kind" in motion.final)
                    ? "final"
                    : undefined
                : undefined;
            const clipBinding =
              phase && motion?.kind === "native-patch-clips" ? motion[phase] : undefined;
            const frames =
              clipBinding && !("kind" in clipBinding) ? clipBinding.material_frames : undefined;
            const asset = await loader.load(
              binding,
              undefined,
              frames?.flatMap((frame) => frame.materials.map((m) => m.material_index)),
            );
            owned.add(asset);
            if (!current()) return false;
            const appearance = captureLoadedStateAppearance(asset);
            let clone: THREE.Object3D;
            if (phase && motion?.kind === "native-patch-clips") {
              const binding = motion[phase];
              if ("kind" in binding || !appearance)
                throw new Error("Patch clip requires its pinned animated model");
              if (appearance.clips.length !== 1 || appearance.clips[0]!.name !== binding.clip)
                throw new Error("Patch clip identity differs from model");
              if (
                appearance.clips[0]!.tracks.some(
                  (track) => track.track.times.at(-1)! < binding.duration_ticks,
                )
              )
                throw new Error("Patch clip does not cover source phase duration");
              const player = new StateAppearancePlayer(appearance, true);
              const phases = this.clips.get(family.id) ?? {};
              phases[phase] = player;
              this.clips.set(family.id, phases);
              player.select(
                binding.clip,
                phase === "final"
                  ? { mode: "loop", cycleTicks: binding.duration_ticks }
                  : { mode: "clamp", terminalTick: binding.duration_ticks },
              );
              if (frames)
                this.materialFrames.set(
                  player,
                  new StateMaterialFrames(asset, player.content, frames),
                );
              clone = player.object;
            } else {
              if (appearance)
                throw new Error(
                  "Physical endpoint contains animation; endpoint-only contract required",
                );
              clone = asset.clone(true);
            }
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
        if (family.physical.motion?.kind === "native-element-component-poses") {
          const components = new ComponentNodePoses(pair.applied, family.physical.motion);
          this.componentPoses.set(family.id, components);
          pair.frames.push(...components.roots.values());
        }
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
    if (
      !this.ready ||
      !["native-art", "native-remainder", "physical-endpoint", "physical-motion"].includes(mode)
    )
      throw new Error("State delivery mode is unavailable");
    if (mode === "native-remainder" && !this.family(this.selectedFamily!).native_remainder)
      throw new Error("Selected family has no reviewed native remainder");
    if (mode === "physical-motion" && !this.family(this.selectedFamily!).physical.motion)
      throw new Error("Selected family has no reviewed physical motion");
    const previousPose = this.motionEndpoint;
    this.currentMode = mode;
    this.physical.visible = mode === "physical-endpoint" || mode === "physical-motion";
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
    this.selectedEndpoint = endpoint;
    for (const poses of this.componentPoses.values()) poses.reset();
    for (const poses of this.nodePoses.values()) poses.reset();
    for (const phases of this.clips.values())
      for (const player of Object.values(phases)) {
        player.sampleExternalTick(0);
        this.materialFrames.get(player)?.sample(player.tick);
      }
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
    if (
      family.physical.motion?.kind === "native-element-stage-poses" ||
      family.physical.motion?.kind === "native-element-component-poses"
    )
      throw new Error("Staged motion requires independent source clocks");
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
    if (
      playing &&
      this.selectedFamily &&
      ["native-element-stage-poses", "native-element-component-poses"].includes(
        this.family(this.selectedFamily).physical.motion?.kind ?? "",
      )
    )
      throw new Error("Staged motion requires independent source clocks");
    this.native.setPlaying(playing);
  }
  private sampleMotion() {
    if (this.mode !== "physical-motion" || !this.selectedFamily) return;
    const pose =
      this.sourceClocks?.physicalPose(this.selectedFamily) ??
      physicalPatchPose(this.contract!, this.selectedFamily, this.familyTick(this.selectedFamily));
    const pair = this.roots.get(this.selectedFamily)!;
    const components = this.componentPoses.get(this.selectedFamily);
    if (components) {
      components.apply(pose.component_frames ?? []);
      pair.initial.visible = pose.endpoint === "initial" && pose.visible !== false;
      pair.applied.visible = false;
      return;
    }
    if (pose.clip_phase !== undefined) {
      const player = this.clips.get(this.selectedFamily)?.[pose.clip_phase];
      if (!player || pose.clip_tick === undefined) throw new Error("Patch clip player missing");
      player.sampleExternalTick(pose.clip_tick);
      this.materialFrames.get(player)?.sample(player.tick);
    }
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
  /** Physical roots drawn with each currently bound remainder frame, keyed by element. */
  remainderPhysicalRoots(): Map<string, THREE.Object3D> {
    if (this.mode !== "native-remainder" || !this.selectedFamily)
      throw new Error("Native remainder mode is not selected");
    const roots = new Map<string, THREE.Object3D>();
    const pending: { element: string; family: string; nodeFrame: number | undefined }[] = [];
    const componentSelections = new Map<
      string,
      { component_id: string; element_id: string; node_frame: number }[]
    >();
    for (const selected of this.native.remainderSelection()) {
      const family = this.contract!.families.find((f) =>
        f.element_ids.includes(selected.element_id),
      )!;
      const frame = nativeRemainderFrame(family, selected.element_id, selected.frame);
      if (!frame) throw new Error("Native remainder frame binding missing");
      if (family.physical.motion?.kind === "native-element-component-poses") {
        const selections = componentSelections.get(family.id) ?? [];
        const current = nativeRemainderComponentFrames(
          family,
          selected.element_id,
          selected.frame,
          frame.physical,
        );
        if (current.some((c) => selections.some((s) => s.component_id === c.component_id)))
          throw new Error("Overlapping native remainder component owners");
        selections.push(...current);
        componentSelections.set(family.id, selections);
        roots.set(
          selected.element_id,
          this.componentPoses.get(family.id)!.roots.get(selected.element_id)!,
        );
        continue;
      }
      const nodeFrame = nativeRemainderNodeFrame(
        family,
        selected.element_id,
        selected.frame,
        frame.physical,
      );
      if (family.physical.motion && pending.some((p) => p.family === family.id))
        throw new Error("Overlapping native remainder assembly owners");
      pending.push({ element: selected.element_id, family: family.id, nodeFrame });
    }
    // Validate the entire selection before changing any shared assembly pose.
    for (const [id, poses] of this.componentPoses) poses.apply(componentSelections.get(id) ?? []);
    for (const item of pending) {
      if (item.nodeFrame !== undefined) {
        const poses = this.nodePoses.get(item.family);
        if (!poses) throw new Error("Reviewed remainder node pose binding missing");
        poses.apply(item.nodeFrame);
      }
      roots.set(item.element, this.roots.get(item.family)!.applied);
    }
    return roots;
  }
  /** For a target clip family in 3D motion: "native" while its target still draws native frames. */
  get elementClipPhase(): "native" | "physical" | undefined {
    if (this.mode !== "physical-motion" || !this.selectedFamily) return undefined;
    if (this.family(this.selectedFamily).physical.motion?.kind !== "native-element-clip")
      return undefined;
    const pose =
      this.sourceClocks?.physicalPose(this.selectedFamily) ??
      physicalPatchPose(this.contract!, this.selectedFamily, this.familyTick(this.selectedFamily));
    return pose.native_element ? "native" : "physical";
  }
  /** Linked compiled transitions of the selected family and the endpoint shown in 3D, if any. */
  get gameplayLink():
    | { family: string; endpoint: "initial" | "applied" | "none"; transitions: string[] }
    | undefined {
    if (!this.contract || !this.selectedFamily) return undefined;
    const family = this.family(this.selectedFamily);
    return {
      family: family.id,
      endpoint:
        this.mode === "physical-motion"
          ? family.gameplay_transition_ids?.length
            ? (this.sourceClocks?.gameplayEndpoint(family.id) ?? "none")
            : this.motionEndpoint!
          : this.mode === "physical-endpoint"
            ? this.selectedEndpoint
            : "none",
      transitions: [...(family.gameplay_transition_ids ?? [])],
    };
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
