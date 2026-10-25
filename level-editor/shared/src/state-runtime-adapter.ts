import type { StateDeliveryContract } from "./state-delivery.ts";
import {
  StateEventClocks,
  type StateClockBatch,
  type StateClockSample,
} from "./state-event-clocks.ts";
import { sourceSpriteTick, type SourceSpriteClock } from "./source-sprite-clock.ts";

type Source = StateClockSample["source"];
/** Prepared from the native construction and animation-row inventory, never guessed from display order. */
export type StateRuntimeBinding = {
  id: string;
  source: Source;
  runtime_index: number;
} & (
  | {
      kind: "element";
      initial_action: string;
      initial_row: number;
      action: string;
      action_row: number;
      /** Reviewed leading presentation frames absent from the runtime action row. */
      action_frame_offset?: number;
      /** Required when the initial and triggered actions share the same row/action. */
      progression?: { initial: number; action: number };
    }
  | {
      kind: "patch" | "background";
      phase_rows: { initial?: number; forward?: number; applied?: number };
    }
);
export interface RuntimeSpriteObservation extends SourceSpriteClock {
  active: boolean;
  row: number;
  last_action: string;
  target_progression?: number;
}
export interface RuntimeStateObservation {
  mission: string;
  engine_frame: number;
  owners: { index: number; sprite: RuntimeSpriteObservation }[];
  patches: {
    index: number;
    applied: boolean;
    in_transition: boolean;
    sprite: RuntimeSpriteObservation | null;
  }[];
}

/** Bridges source-qualified runtime identities to a loaded delivery contract's independent clocks. */
export class StateRuntimeAdapter {
  private readonly contract: StateDeliveryContract;
  private readonly bindings: StateRuntimeBinding[];
  private readonly clocks: StateEventClocks;
  private sequence = 0;

  constructor(contract: StateDeliveryContract, bindings: StateRuntimeBinding[]) {
    this.contract = structuredClone(contract);
    this.bindings = structuredClone(bindings);
    this.clocks = new StateEventClocks(contract);
    const required = new Set(
      contract.families.flatMap((f) => [
        ...f.element_ids.map((id) => `element:${id}`),
        ...(f.hidden_initial_element_ids ?? []).map((id) => `element:${id}`),
        ...(f.patch_ids ?? []).map((id) => `patch:${id}`),
        ...f.background_ids.map((id) => `background:${id}`),
      ]),
    );
    const seen = new Set<string>();
    for (const b of this.bindings) {
      const key = `${b.kind}:${b.id}`;
      const rows =
        b.kind === "element"
          ? contract.native.elements
          : b.kind === "patch"
            ? contract.native.patch_states
            : contract.native.background_states;
      const row = rows?.find((r) => r.id === b.id);
      if (
        !required.has(key) ||
        seen.has(key) ||
        !row ||
        row.source.kind !== b.source.kind ||
        row.source.index !== b.source.index ||
        row.source.sha256 !== b.source.sha256 ||
        !Number.isSafeInteger(b.runtime_index) ||
        b.runtime_index < 0
      )
        throw new Error(`Invalid prepared state runtime binding: ${key}`);
      if (b.kind === "element") {
        if (
          !b.action ||
          !b.initial_action ||
          (b.action === b.initial_action && b.progression === undefined) ||
          !Number.isSafeInteger(b.initial_row) ||
          b.initial_row < 0 ||
          !Number.isSafeInteger(b.action_row) ||
          b.action_row < 0
        )
          throw new Error("Missing prepared target action row");
        if (
          b.progression !== undefined &&
          (!b.progression ||
            Object.keys(b.progression).sort().join(",") !== "action,initial" ||
            !Number.isSafeInteger(b.progression.initial) ||
            b.progression.initial < 0 ||
            !Number.isSafeInteger(b.progression.action) ||
            b.progression.action < 0 ||
            b.progression.initial === b.progression.action)
        )
          throw new Error("Invalid prepared target progression");
        if (
          b.action_frame_offset !== undefined &&
          (!Number.isSafeInteger(b.action_frame_offset) ||
            b.action_frame_offset < 0 ||
            !("frames" in row) ||
            b.action_frame_offset >= row.frames.length)
        )
          throw new Error("Invalid prepared target frame offset");
      } else if (Object.values(b.phase_rows).some((n) => !Number.isSafeInteger(n) || n < 0))
        throw new Error("Invalid prepared patch phase row");
      seen.add(key);
    }
    if (seen.size !== required.size) throw new Error("Incomplete prepared state runtime inventory");
  }

  sample(observed: RuntimeStateObservation): StateClockBatch {
    if (observed.mission !== this.contract.native.mission)
      throw new Error("Runtime observation belongs to another mission");
    if (
      new Set(observed.owners.map((r) => r.index)).size !== observed.owners.length ||
      new Set(observed.patches.map((r) => r.index)).size !== observed.patches.length
    )
      throw new Error("Ambiguous runtime observation identities");
    const samples: StateClockSample[] = this.bindings.map((b) => {
      if (b.kind === "element") {
        const raw = observed.owners.find((r) => r.index === b.runtime_index)?.sprite;
        if (!raw || typeof raw.active !== "boolean")
          throw new Error(`Missing runtime owner ${b.runtime_index}`);
        if (raw.last_action !== b.action && raw.last_action !== b.initial_action)
          throw new Error("Unbound runtime owner action");
        let active = raw.last_action === b.action;
        if (b.progression) {
          if (
            raw.target_progression !== b.progression.initial &&
            raw.target_progression !== b.progression.action
          )
            throw new Error("Unbound runtime target progression");
          active = raw.target_progression === b.progression.action;
          if (raw.last_action !== (active ? b.action : b.initial_action))
            throw new Error("Runtime target action differs from its progression");
        }
        if (raw.row !== (active ? b.action_row : b.initial_row))
          throw new Error("Runtime owner action row differs");
        const e = this.contract.native.elements.find((e) => e.id === b.id)!;
        // Hidden initial-only artwork has no action-frame clock to sample.
        const offset = b.action_frame_offset ?? 0;
        const tick =
          active && e.frames.length
            ? e.frames.slice(0, offset).reduce((sum, frame) => sum + frame.delay + 1, 0) +
              sourceSpriteTick(e.frames.slice(offset), raw)
            : 0;
        return { kind: "element", id: b.id, source: b.source, active, visible: raw.active, tick };
      }
      const raw = observed.patches.find((p) => p.index === b.runtime_index);
      if (!raw || typeof raw.applied !== "boolean" || typeof raw.in_transition !== "boolean")
        throw new Error(`Missing runtime patch ${b.runtime_index}`);
      if (raw.applied && raw.in_transition)
        throw new Error("Reverse runtime patch clocks need a separately prepared reverse binding");
      const phase = raw.in_transition ? "forward" : raw.applied ? "applied" : "initial";
      const states =
        b.kind === "patch"
          ? this.contract.native.patch_states
          : this.contract.native.background_states;
      const state = states!.find((p) => p.id === b.id)!;
      const frames =
        phase === "initial" ? state.initial : phase === "forward" ? state.transition : state.final;
      let tick = 0;
      if (frames.length) {
        if (
          !raw.sprite ||
          b.phase_rows[phase] === undefined ||
          raw.sprite.row !== b.phase_rows[phase]
        )
          throw new Error("Runtime patch phase row differs from its prepared source");
        tick = sourceSpriteTick(frames, raw.sprite);
      }
      // An empty phase has no animation clock; zero is its neutral presentation coordinate.
      return { kind: b.kind, id: b.id, source: b.source, phase, tick };
    });
    const n = this.contract.native;
    const batch = {
      mission: n.mission,
      mission_data_sha256: n.mission_data_sha256,
      level_data_sha256: n.level_data_sha256,
      sequence: this.sequence,
      simulation_tick: observed.engine_frame,
      samples,
    };
    this.clocks.apply(batch);
    this.sequence++;
    return batch;
  }
}
