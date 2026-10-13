import {
  nativeBackgroundFrames,
  nativePatchBackgroundFrame,
  nativeTransientPatchFrame,
  type NativeBackgroundPhase,
} from "./native-state-presentation.ts";
import type { StateDeliveryContract } from "./state-delivery.ts";

type Source = StateDeliveryContract["native"]["elements"][number]["source"];
export type StateClockSample =
  | { kind: "element"; id: string; source: Source; active: boolean; tick: number }
  | {
      kind: "patch" | "background";
      id: string;
      source: Source;
      phase: NativeBackgroundPhase;
      tick: number;
    };

/** Observed source clocks, not elapsed time since the containing family was selected. */
export interface StateClockBatch {
  mission: string;
  mission_data_sha256: string;
  level_data_sha256: string;
  sequence: number;
  simulation_tick: number;
  samples: StateClockSample[];
}

/** An omitted owner keeps its last sample. Only an explicit event can advance or reset it. */
export class StateEventClocks {
  private readonly contract: StateDeliveryContract;
  private clocks = new Map<string, StateClockSample>();
  private sequence = -1;
  private simulationTick = -1;

  constructor(contract: StateDeliveryContract) {
    this.contract = structuredClone(contract);
  }

  apply(batch: StateClockBatch): StateClockSample[] {
    const n = this.contract.native;
    if (
      batch.mission !== n.mission ||
      batch.mission_data_sha256 !== n.mission_data_sha256 ||
      batch.level_data_sha256 !== n.level_data_sha256
    )
      throw new Error("State event source does not match the loaded mission");
    if (
      !Number.isSafeInteger(batch.sequence) ||
      batch.sequence <= this.sequence ||
      !Number.isSafeInteger(batch.simulation_tick) ||
      batch.simulation_tick < 0 ||
      batch.simulation_tick < this.simulationTick ||
      !Array.isArray(batch.samples)
    )
      throw new Error("Invalid or out-of-order state event batch");
    const keys = new Set<string>();
    for (const s of batch.samples) {
      const key = `${s.kind}:${s.id}`;
      if (keys.has(key)) throw new Error("Duplicate state clock in event batch");
      keys.add(key);
      const rows =
        s.kind === "element"
          ? n.elements
          : s.kind === "patch"
            ? n.patch_states
            : s.kind === "background"
              ? n.background_states
              : undefined;
      const row = rows?.find((r) => r.id === s.id);
      if (
        !row ||
        !s.source ||
        row.source.kind !== s.source.kind ||
        row.source.index !== s.source.index ||
        row.source.sha256 !== s.source.sha256
      )
        throw new Error(`Unbound state event source: ${key}`);
      if (!Number.isSafeInteger(s.tick) || s.tick < 0)
        throw new Error("Invalid source animation tick");
      if (s.kind === "element") {
        if (typeof s.active !== "boolean") throw new Error("Invalid source element activity");
      } else {
        if (
          !["initial", "forward", "applied", "reverse"].includes(s.phase) ||
          (s.phase === "reverse" && "definitive" in row && row.definitive)
        )
          throw new Error("Invalid source patch phase");
        if (s.kind === "patch") {
          const patch = n.patch_states!.find((p) => p.id === s.id)!;
          nativePatchBackgroundFrame(patch, s.phase, s.tick);
          nativeTransientPatchFrame(patch, s.phase, s.tick);
        } else
          nativeBackgroundFrames(
            n.background_states!.find((p) => p.id === s.id)!,
            s.phase,
            s.tick,
          );
        // Reverse physical tracks require their own reviewed interpretation.
        if (
          s.phase === "reverse" &&
          this.contract.families.some((f) => f.physical.motion?.patch_id === s.id)
        )
          throw new Error("Reviewed physical motion does not support reverse source events");
      }
    }
    const samples = structuredClone(batch.samples);
    for (const s of samples) this.clocks.set(`${s.kind}:${s.id}`, s);
    this.sequence = batch.sequence;
    this.simulationTick = batch.simulation_tick;
    return structuredClone(samples);
  }

  sample(kind: StateClockSample["kind"], id: string): StateClockSample | undefined {
    const sample = this.clocks.get(`${kind}:${id}`);
    return sample ? structuredClone(sample) : undefined;
  }

  /** Physical tracks use their own patch clock; final animation tick is a separate clock. */
  physicalTick(familyId: string): number | undefined {
    const f = this.contract.families.find((f) => f.id === familyId);
    if (!f) throw new Error(`Unknown state family: ${familyId}`);
    if (!f.physical.motion) return undefined;
    const sample = this.sample("patch", f.physical.motion.patch_id);
    if (!sample || sample.kind === "element" || sample.phase === "initial") return undefined;
    if (sample.phase === "reverse") throw new Error("Unreviewed reverse physical motion");
    return sample.phase === "applied" ? f.body_terminal_tick : sample.tick;
  }
}
