import { verifyNativePresentationSource } from "./native-state-presentation.ts";
import { resolveMissionStateTargets, type MissionStateSource } from "./mission-state-layer.ts";
import type { MissionStateContract } from "../../shared/src/mission-state.ts";
import {
  nativePresentationFrame,
  type NativeStatePresentationContract,
} from "../../shared/src/native-state-presentation.ts";
import {
  SourceAnimationClocks,
  type SourceClockHandle,
} from "../../shared/src/native-animation-clocks.ts";

/** Explicit external-clock port. Its implementation must not advance another elapsed-time cursor. */
export interface PhysicalTickConsumer {
  internallyPlaying(): boolean;
  sampleExternalTick(tick: number): void;
}

/** Exact loop-contract linkage. Resource decoding begins only after set() publishes every identity. */
export class SourceContractClockBinding {
  private readonly clocks = new SourceAnimationClocks();
  private request = 0;
  private disposed = false;
  private native: NativeStatePresentationContract | undefined;
  private physical: MissionStateContract | undefined;
  private handles = new Map<string, SourceClockHandle>();
  private physicalIds = new Map<string, string>();
  private consumers = new Map<string, PhysicalTickConsumer>();

  clear() {
    if (this.disposed) throw new Error("Source contract binding is disposed");
    this.request++;
    this.clocks.clear();
    this.native = undefined;
    this.physical = undefined;
    this.handles.clear();
    this.physicalIds.clear();
    this.consumers.clear();
  }
  async set(
    native: NativeStatePresentationContract,
    physical: MissionStateContract,
    source: MissionStateSource,
  ) {
    this.clear();
    const request = this.request,
      n = structuredClone(native),
      p = structuredClone(physical),
      s = structuredClone(source);
    const current = () => !this.disposed && request === this.request;
    try {
      await verifyNativePresentationSource(n, s);
      await resolveMissionStateTargets(p, s);
      if (!current()) return false;
      if (
        n.mission !== p.mission ||
        n.level_data_sha256 !== p.level_data_sha256 ||
        n.mission_data_sha256 !== p.mission_data_sha256 ||
        n.camera_elevation_deg !== p.camera_elevation_deg
      )
        throw new Error("Physical/native source contracts differ");
      if (
        n.background_states?.length ||
        n.patch_states?.length ||
        n.elements.some((e) => e.frames.length && !e.loop)
      )
        throw new Error("Loop clock binding cannot stand in for patch activation/state timing");
      const ids = new Set<string>(),
        slots = new Set<string>();
      for (const e of n.elements) {
        const slot = `${e.source.kind}:${e.source.index}`;
        if (ids.has(e.id) || slots.has(slot))
          throw new Error("Duplicate native clock identity/source slot");
        ids.add(e.id);
        slots.add(slot);
      }
      const physicalIds = new Map<string, string>();
      for (const t of p.targets) {
        const e = n.elements.find(
          (e) => e.source.kind === "mission-target" && e.source.index === t.target_index,
        );
        if (
          !e ||
          !e.frames.length ||
          e.source.sha256 !== t.target_sha256 ||
          t.representation !== "physical"
        )
          throw new Error(`Physical target has no exact native clock identity: ${t.id}`);
        const cycle = e.frames.reduce((sum, f) => sum + f.delay + 1, 0);
        if (t.actions.some((a) => a.timing.mode !== "loop" || a.timing.cycleTicks !== cycle))
          throw new Error(`Physical/native cycle differs: ${t.id}`);
        physicalIds.set(t.id, e.id);
      }
      for (const e of n.elements)
        if (
          e.source.kind === "mission-target" &&
          e.frames.length &&
          ![...physicalIds.values()].includes(e.id)
        )
          throw new Error(`Missing physical target for native identity: ${e.id}`);
      // All definitions and source-slot checks complete synchronously before callers can start loads.
      this.clocks.reset({ levelSha256: n.level_data_sha256, missionSha256: n.mission_data_sha256 });
      for (const e of n.elements) {
        const handle = this.clocks.define({
          kind: e.source.kind,
          index: e.source.index,
          sourceSha256: e.source.sha256,
        });
        this.handles.set(e.id, handle);
        this.clocks.setState(handle, { active: e.active, playing: false });
      }
      this.native = n;
      this.physical = p;
      this.physicalIds = physicalIds;
      return true;
    } catch (error) {
      if (!current()) return false;
      this.clear();
      throw error;
    }
  }
  /** Capture before an async load, and check lifetime again before attaching its representation. */
  resourceToken(id: string) {
    return this.handle(id);
  }
  resourceIsCurrent(token: SourceClockHandle) {
    return this.clocks.isCurrent(token);
  }
  attachPhysical(id: string, consumer: PhysicalTickConsumer, token: SourceClockHandle) {
    const nativeId = this.physicalIds.get(id);
    if (!nativeId || this.handles.get(nativeId) !== token || !this.resourceIsCurrent(token))
      throw new Error("Retired or mismatched physical resource binding");
    if (consumer.internallyPlaying())
      throw new Error("Physical consumer would double-advance its clock");
    if (this.consumers.has(id)) throw new Error("Physical consumer already attached");
    consumer.sampleExternalTick(this.physicalSampleTick(id));
    this.consumers.set(id, consumer);
  }
  get ready() {
    return this.native !== undefined;
  }
  detachPhysical(id: string) {
    this.consumers.delete(id);
  }
  seekPhysical(id: string, tick: number) {
    const nativeId = this.physicalIds.get(id);
    if (!nativeId) throw new Error(`Unknown physical clock: ${id}`);
    this.seek(nativeId, tick);
  }
  setPhysicalPlaying(id: string, playing: boolean) {
    const nativeId = this.physicalIds.get(id);
    if (!nativeId) throw new Error(`Unknown physical clock: ${id}`);
    this.setPlaying(nativeId, playing);
  }
  setAllPlaying(playing: boolean) {
    for (const id of this.handles.keys()) this.setPlaying(id, playing);
  }
  physicalToken(id: string) {
    const nativeId = this.physicalIds.get(id);
    if (!nativeId) throw new Error(`Unknown physical clock: ${id}`);
    return this.handle(nativeId);
  }
  snapshot() {
    if (!this.native) throw new Error("Source contract clocks are not ready");
    return this.native.elements.map((e) => {
      const cursor = this.clocks.read(this.handle(e.id));
      return Object.freeze({
        id: e.id,
        source: Object.freeze({ ...e.source }),
        ...cursor,
        frame: e.frames.length ? nativePresentationFrame(e, cursor.tick) : -1,
      });
    });
  }
  setPlaying(id: string, playing: boolean) {
    this.clocks.setState(this.handle(id), { playing });
  }
  seek(id: string, tick: number) {
    this.assertExternalConsumers();
    this.clocks.seek(this.handle(id), tick);
    this.samplePhysical();
  }
  advance(sequence: number, seconds: number) {
    // A rogue internally-playing representation must fail before the shared cursor moves.
    this.assertExternalConsumers();
    const count = this.clocks.advance(sequence, seconds);
    this.samplePhysical();
    return count;
  }
  samplePhysical() {
    this.assertExternalConsumers();
    for (const [id, consumer] of this.consumers)
      consumer.sampleExternalTick(this.physicalSampleTick(id));
  }
  dispose() {
    if (!this.disposed) {
      this.clear();
      this.clocks.dispose();
      this.disposed = true;
    }
  }
  private assertExternalConsumers() {
    for (const consumer of this.consumers.values())
      if (consumer.internallyPlaying())
        throw new Error("Physical consumer would double-advance its clock");
  }
  private physicalSampleTick(id: string) {
    const timing = this.physical?.targets.find((t) => t.id === id)?.actions[0]?.timing;
    if (!timing || timing.mode !== "loop")
      throw new Error(`Missing validated physical loop timing: ${id}`);
    // Keep bounded animation sampling independent of the long-lived authoritative source cursor.
    return this.clocks.read(this.physicalToken(id)).tick % timing.cycleTicks;
  }
  private handle(id: string) {
    const handle = this.handles.get(id);
    if (!handle || !this.native) throw new Error(`Missing source clock identity: ${id}`);
    return handle;
  }
}
