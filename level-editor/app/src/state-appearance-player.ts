import * as THREE from "three";
import { clone as cloneHierarchy } from "three/examples/jsm/utils/SkeletonUtils.js";

const HZ = 25;
const MAX_TICK = 2 ** 24 - 1;
const properties = new Set(["position", "quaternion", "scale", "morphTargetInfluences"]);

export type StateAppearanceTiming =
  | { mode: "loop"; cycleTicks: number }
  | { mode: "clamp"; terminalTick: number };

type CapturedTrack = {
  sourceNode: THREE.Object3D;
  childPath: readonly number[];
  property: string;
  track: THREE.KeyframeTrack;
};
type CapturedClip = { name: string; tracks: readonly CapturedTrack[] };
export type StateAppearanceTemplate = {
  /** Capture before restoring display names; names may subsequently change. */
  readonly sourceRoot: THREE.Object3D;
  readonly clips: readonly CapturedClip[];
};

function integerTick(value: number, label: string) {
  if (!Number.isSafeInteger(value) || value < 0 || value > MAX_TICK)
    throw new Error(`${label} must be an integer between 0 and ${MAX_TICK}`);
  return value;
}

function pathFrom(root: THREE.Object3D, node: THREE.Object3D): number[] {
  if (node === root) return [];
  if (!node.parent) throw new Error(`Animation target is outside selected asset: ${node.name}`);
  const index = node.parent.children.indexOf(node);
  if (index < 0) throw new Error("Animation hierarchy is inconsistent");
  return [...pathFrom(root, node.parent), index];
}

function atPath(root: THREE.Object3D, path: readonly number[]) {
  let node = root;
  for (const index of path) {
    const child = node.children[index];
    if (!child) throw new Error("Captured animation hierarchy changed");
    node = child;
  }
  return node;
}

/** Bind loader track names while they still exist, before name restoration or group extraction. */
export function captureStateAppearance(
  loadedRoot: THREE.Object3D,
  clips: readonly THREE.AnimationClip[],
  selectedRoot: THREE.Object3D = loadedRoot,
): StateAppearanceTemplate {
  pathFrom(loadedRoot, selectedRoot);
  const nodes: THREE.Object3D[] = [];
  loadedRoot.traverse((node) => nodes.push(node));
  const names = new Set<string>();
  const captured = clips.map((clip): CapturedClip => {
    if (!clip.name || names.has(clip.name)) throw new Error("Animation clip names must be unique");
    names.add(clip.name);
    if (clip.blendMode !== THREE.NormalAnimationBlendMode)
      throw new Error(`Additive state appearance clips are unsupported: ${clip.name}`);
    if (!clip.tracks.length) throw new Error(`Animation clip has no tracks: ${clip.name}`);
    const bindings = new Set<string>();
    const tracks = clip.tracks.map((original): CapturedTrack => {
      const parsed = THREE.PropertyBinding.parseTrackName(original.name);
      if (
        parsed.objectName !== undefined ||
        parsed.objectIndex !== undefined ||
        parsed.propertyIndex !== undefined ||
        !properties.has(parsed.propertyName)
      )
        throw new Error(`Unsupported state appearance track: ${original.name}`);
      const matches =
        !parsed.nodeName || parsed.nodeName === "."
          ? [loadedRoot]
          : nodes.filter((node) => node.uuid === parsed.nodeName || node.name === parsed.nodeName);
      if (matches.length !== 1)
        throw new Error(`Missing or ambiguous animation target: ${original.name}`);
      const sourceNode = matches[0]!;
      const childPath = pathFrom(selectedRoot, sourceNode);
      const binding = `${sourceNode.uuid}.${parsed.propertyName}`;
      if (bindings.has(binding)) throw new Error(`Duplicate animation binding: ${binding}`);
      bindings.add(binding);
      const track = original.clone();
      if (!track.times.length || track.times[0] !== 0)
        throw new Error(`State appearance tracks must begin at tick zero: ${original.name}`);
      let previous = -1;
      const ticks = Array.from(track.times, (time) => {
        const tick = integerTick(Math.round(time * HZ), "Animation key tick");
        if (Math.abs(time * HZ - tick) > 0.001 || tick <= previous)
          throw new Error(`Animation keys must follow the 25 Hz grid: ${original.name}`);
        previous = tick;
        return tick;
      });
      // Integer key times avoid float32 seconds selecting the previous STEP frame at a boundary.
      track.times = new Float32Array(ticks);
      if (Array.from(track.values).some((value) => !Number.isFinite(value)))
        throw new Error(`Animation values are not finite: ${original.name}`);
      const size = track.getValueSize();
      const expected =
        parsed.propertyName === "quaternion"
          ? 4
          : parsed.propertyName === "morphTargetInfluences"
            ? (sourceNode as THREE.Mesh).morphTargetInfluences?.length
            : 3;
      if (!expected || size !== expected)
        throw new Error(`Animation value size differs from its target: ${original.name}`);
      return { sourceNode, childPath, property: parsed.propertyName, track };
    });
    return { name: clip.name, tracks };
  });
  if (!captured.length) throw new Error("State appearance requires an animation clip");
  return { sourceRoot: selectedRoot, clips: captured };
}

/** An isolated preview instance. This does not implement mission activation or scene draw order. */
export class StateAppearancePlayer {
  /** Editable placement stays outside the animated hierarchy. */
  readonly object = new THREE.Group();
  readonly content: THREE.Object3D;
  readonly clipNames: readonly string[];
  private readonly mixer: THREE.AnimationMixer;
  private readonly clips = new Map<string, THREE.AnimationClip>();
  private action: THREE.AnimationAction | undefined;
  private timing: StateAppearanceTiming | undefined;
  private cursor = 0;
  private running = false;
  private disposed = false;

  readonly externallyClocked: boolean;
  constructor(template: StateAppearanceTemplate, externallyClocked = false) {
    this.externallyClocked = externallyClocked;
    this.content = cloneHierarchy(template.sourceRoot);
    this.object.add(this.content);
    for (const captured of template.clips) {
      const tracks = captured.tracks.map((binding) => {
        if (atPath(template.sourceRoot, binding.childPath) !== binding.sourceNode)
          throw new Error("Captured animation hierarchy changed");
        const target = atPath(this.content, binding.childPath);
        const track = binding.track.clone();
        track.name = `${target.uuid}.${binding.property}`;
        return track;
      });
      this.clips.set(captured.name, new THREE.AnimationClip(captured.name, -1, tracks));
    }
    this.clipNames = Object.freeze([...this.clips.keys()]);
    this.mixer = new THREE.AnimationMixer(this.content);
  }

  get tick() {
    return Math.floor(this.cursor + 1e-9);
  }
  get playing() {
    return this.running;
  }

  select(name: string, timing: StateAppearanceTiming) {
    this.assertAlive();
    const clip = this.clips.get(name);
    if (!clip) throw new Error(`Unknown state appearance clip: ${name}`);
    const end = integerTick(
      timing.mode === "loop" ? timing.cycleTicks : timing.terminalTick,
      "State appearance endpoint",
    );
    if (timing.mode === "loop" && end === 0) throw new Error("Loop cycle must be positive");
    this.mixer.stopAllAction();
    this.timing = { ...timing };
    this.cursor = 0;
    this.running = false;
    clip.duration = Math.max(clip.duration, end, 1);
    this.action = this.mixer.clipAction(clip);
    this.action.setLoop(THREE.LoopOnce, 1);
    this.action.clampWhenFinished = true;
    this.sample();
  }

  seek(tick: number) {
    this.assertInternalClock();
    this.assertSelected();
    integerTick(tick, "Seek tick");
    this.cursor =
      this.timing!.mode === "loop"
        ? tick % this.timing!.cycleTicks
        : Math.min(tick, this.timing!.terminalTick);
    if (this.timing!.mode === "clamp" && this.tick === this.timing!.terminalTick)
      this.running = false;
    this.sample();
  }

  play() {
    this.assertInternalClock();
    this.assertSelected();
    this.running = this.timing!.mode === "loop" || this.tick < this.timing!.terminalTick;
  }

  pause() {
    this.assertInternalClock();
    this.assertAlive();
    this.running = false;
  }

  advance(seconds: number) {
    this.assertInternalClock();
    this.assertSelected();
    const delta = seconds * HZ;
    if (!Number.isFinite(delta) || delta < 0)
      throw new Error("Elapsed seconds must be finite and nonnegative");
    if (!this.running) return;
    if (this.timing!.mode === "loop") {
      this.cursor = (this.cursor + delta) % this.timing!.cycleTicks;
      if (this.tick >= this.timing!.cycleTicks) this.cursor = 0;
    } else {
      this.cursor = Math.min(this.cursor + delta, this.timing!.terminalTick);
      if (this.tick >= this.timing!.terminalTick) {
        this.cursor = this.timing!.terminalTick;
        this.running = false;
      }
    }
    this.sample();
  }

  /** Sample an authoritative source cursor without acquiring play/pause ownership. */
  sampleExternalTick(tick: number) {
    this.assertSelected();
    if (!this.externallyClocked) throw new Error("Player does not use an external clock");
    if (!Number.isSafeInteger(tick) || tick < 0) throw new Error("Invalid external source tick");
    this.cursor =
      this.timing!.mode === "loop"
        ? tick % this.timing!.cycleTicks
        : Math.min(tick, this.timing!.terminalTick);
    this.sample();
  }

  private assertInternalClock() {
    if (this.externallyClocked) throw new Error("External-clock player cannot own time controls");
  }

  dispose() {
    if (this.disposed) return;
    this.running = false;
    this.mixer.stopAllAction();
    this.mixer.uncacheRoot(this.content);
    this.disposed = true;
    // Geometry and materials are shared with library assets and remain owned by their loader.
  }

  private sample() {
    this.action!.reset().play();
    this.mixer.setTime(this.tick);
    this.content.updateMatrixWorld(true);
  }
  private assertAlive() {
    if (this.disposed) throw new Error("State appearance player is disposed");
  }
  private assertSelected() {
    this.assertAlive();
    if (!this.action || !this.timing) throw new Error("Select a state appearance clip first");
  }
}
