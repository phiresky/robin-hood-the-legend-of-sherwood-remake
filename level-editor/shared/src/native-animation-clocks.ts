export interface SourceClockContext {
  levelSha256: string;
  missionSha256: string;
}
export interface SourceClockIdentity {
  kind: "map-animation" | "mission-target" | "map-patch" | "mission-patch" | "actor" | "pickup";
  index: number;
  sourceSha256: string;
}
export interface SourceClockHandle {
  readonly epoch: number;
  readonly key: string;
  readonly generation: number;
}
export interface SourceClockSnapshot extends SourceClockHandle {
  readonly tick: number;
  readonly active: boolean;
  readonly playing: boolean;
}
type Entry = {
  handle: SourceClockHandle;
  tick: number;
  active: boolean;
  playing: boolean;
};
const kinds = new Set([
  "map-animation",
  "mission-target",
  "map-patch",
  "mission-patch",
  "actor",
  "pickup",
]);
function hash(value: string) {
  if (typeof value !== "string" || !/^[0-9a-f]{64}$/.test(value))
    throw new Error("Invalid source clock hash");
}
function tick(value: number) {
  if (!Number.isSafeInteger(value) || value < 0) throw new Error("Invalid source clock integer");
}

/** One simulation tick boundary, independently controlled source cursors, read-only consumers. */
export class SourceAnimationClocks {
  private epoch = 0;
  private generation = 0;
  private context: Readonly<SourceClockContext> | undefined;
  private entries = new Map<string, Entry>();
  private slots = new Map<string, string>();
  private frame = -1;
  private wholeTicks = 0;
  private fraction = 0;
  private disposed = false;

  /** Replacing a mission or level retires all outstanding handles and resource-load tokens. */
  reset(context: SourceClockContext) {
    this.alive();
    hash(context.levelSha256);
    hash(context.missionSha256);
    this.clear();
    this.context = Object.freeze({ ...context });
    return this.epoch;
  }
  clear() {
    this.alive();
    this.epoch++;
    this.context = undefined;
    this.entries.clear();
    this.slots.clear();
    this.frame = -1;
    this.wholeTicks = 0;
    this.fraction = 0;
  }
  /** Definition is idempotent: a later texture/representation load cannot restart the clock. */
  define(identity: SourceClockIdentity): SourceClockHandle {
    this.alive();
    if (!this.context) throw new Error("Source clock context is not loaded");
    if (!kinds.has(identity.kind)) throw new Error("Unknown source clock kind");
    tick(identity.index);
    hash(identity.sourceSha256);
    const slot = `${identity.kind}:${identity.index}`;
    const key = `${this.context.levelSha256}/${this.context.missionSha256}/${slot}/${identity.sourceSha256}`;
    const previous = this.slots.get(slot);
    if (previous !== undefined && previous !== key)
      throw new Error("Source record changed without retiring its clock context");
    const existing = this.entries.get(key);
    if (existing) return existing.handle;
    const handle = Object.freeze({ epoch: this.epoch, key, generation: ++this.generation });
    this.entries.set(key, { handle, tick: 0, active: true, playing: false });
    this.slots.set(slot, key);
    return handle;
  }
  read(handle: SourceClockHandle): SourceClockSnapshot {
    const entry = this.entry(handle);
    return Object.freeze({
      ...entry.handle,
      tick: entry.tick,
      active: entry.active,
      playing: entry.playing,
    });
  }
  /** A seek affects this identity only; it does not reset the shared tick boundary. */
  seek(handle: SourceClockHandle, value: number) {
    tick(value);
    this.entry(handle).tick = value;
  }
  setState(handle: SourceClockHandle, state: { active?: boolean; playing?: boolean }) {
    const entry = this.entry(handle);
    if (
      (state.active !== undefined && typeof state.active !== "boolean") ||
      (state.playing !== undefined && typeof state.playing !== "boolean")
    )
      throw new Error("Invalid source clock state");
    if (state.active !== undefined) entry.active = state.active;
    if (state.playing !== undefined) entry.playing = state.playing;
  }
  /** Called once by the viewport, never once per representation. Sequence is per context epoch. */
  advance(sequence: number, seconds: number): number {
    this.alive();
    if (!this.context) throw new Error("Source clock context is not loaded");
    tick(sequence);
    if (sequence <= this.frame) throw new Error("Duplicate or reversed source clock advance");
    if (!Number.isFinite(seconds) || seconds < 0)
      throw new Error("Invalid source clock elapsed time");
    const total = this.fraction + seconds * 25;
    const step = Math.floor(total + 1e-9);
    tick(step);
    tick(this.wholeTicks + step);
    // Validate every cursor first: a failed advance must not partially update other sources.
    for (const entry of this.entries.values())
      if (entry.active && entry.playing) tick(entry.tick + step);
    for (const entry of this.entries.values())
      if (entry.active && entry.playing) entry.tick += step;
    this.fraction = Math.max(0, total - step);
    this.wholeTicks += step;
    this.frame = sequence;
    return step;
  }
  /** Async resource completion checks lifetime, then reads the current cursor; it never seeks. */
  isCurrent(handle: SourceClockHandle) {
    return (
      !this.disposed &&
      handle.epoch === this.epoch &&
      this.entries.get(handle.key)?.handle === handle
    );
  }
  release(handle: SourceClockHandle) {
    this.entry(handle);
    this.entries.delete(handle.key);
    for (const [slot, key] of this.slots) if (key === handle.key) this.slots.delete(slot);
  }
  timing() {
    this.alive();
    return Object.freeze({
      epoch: this.epoch,
      sequence: this.frame,
      ticks: this.wholeTicks,
      remainderTicks: this.fraction,
    });
  }
  dispose() {
    if (this.disposed) return;
    this.clear();
    this.disposed = true;
  }
  private alive() {
    if (this.disposed) throw new Error("Source animation clocks are disposed");
  }
  private entry(handle: SourceClockHandle) {
    this.alive();
    if (!this.isCurrent(handle)) throw new Error("Retired or foreign source clock handle");
    return this.entries.get(handle.key)!;
  }
}
