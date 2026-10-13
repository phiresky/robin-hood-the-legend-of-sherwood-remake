import test from "node:test";
import assert from "node:assert/strict";
import {
  StateEventClocks,
  type StateClockBatch,
  type StateClockSample,
} from "./state-event-clocks.ts";
import type { StateDeliveryContract } from "./state-delivery.ts";

function fixture() {
  const source = (kind: "mission-patch" | "map-patch" | "mission-target", index: number) => ({
    kind,
    index,
    sha256: String(index % 10).repeat(64),
  });
  const transition = [{ delay: 1 }, { delay: 1 }];
  const body = { id: "owner", source: source("mission-target", 17), frames: transition };
  const ground = {
    id: "receiver",
    source: source("mission-patch", 9),
    transition,
    initial: [],
    final: [],
    definitive: true,
  };
  const patch = {
    id: "motion",
    source: source("map-patch", 6),
    transition,
    initial: [],
    final: [],
    definitive: true,
    integrate_in_background: false,
  };
  const contract = {
    native: {
      mission: "Tac06_FoB_EC",
      mission_data_sha256: "a".repeat(64),
      level_data_sha256: "b".repeat(64),
      elements: [body],
      background_states: [ground],
      patch_states: [patch],
    },
    families: [{ id: "rock", body_terminal_tick: 3, physical: { motion: { patch_id: "motion" } } }],
  } as unknown as StateDeliveryContract;
  const batch = (
    sequence: number,
    simulation_tick: number,
    samples: StateClockSample[],
  ): StateClockBatch => ({
    mission: contract.native.mission,
    mission_data_sha256: contract.native.mission_data_sha256,
    level_data_sha256: contract.native.level_data_sha256,
    sequence,
    simulation_tick,
    samples,
  });
  return { contract, body, ground, patch, batch };
}

test("observed Tac06 rock onset separation cannot advance an unrelated patch clock", () => {
  const f = fixture(),
    clocks = new StateEventClocks(f.contract);
  // Actual lever run observed receiver18 at269 and motion6 at328. The trace does
  // not expose frame counters; these sample ticks exercise the event port only.
  clocks.apply(
    f.batch(0, 269, [
      { kind: "background", id: f.ground.id, source: f.ground.source, phase: "forward", tick: 0 },
    ]),
  );
  assert.equal(clocks.physicalTick("rock"), undefined);
  clocks.apply(
    f.batch(1, 300, [
      { kind: "element", id: f.body.id, source: f.body.source, active: true, tick: 2 },
    ]),
  );
  assert.equal(clocks.physicalTick("rock"), undefined);
  clocks.apply(
    f.batch(2, 328, [
      { kind: "patch", id: f.patch.id, source: f.patch.source, phase: "forward", tick: 0 },
    ]),
  );
  assert.equal(clocks.physicalTick("rock"), 0);
  assert.equal(clocks.sample("background", f.ground.id)!.tick, 0);
  clocks.apply(
    f.batch(3, 330, [
      { kind: "patch", id: f.patch.id, source: f.patch.source, phase: "applied", tick: 0 },
    ]),
  );
  assert.equal(clocks.physicalTick("rock"), 3);
  clocks.apply(
    f.batch(4, 350, [
      { kind: "patch", id: f.patch.id, source: f.patch.source, phase: "applied", tick: 12 },
    ]),
  );
  assert.equal(clocks.physicalTick("rock"), 3);
  assert.equal(clocks.sample("patch", f.patch.id)!.tick, 12);
  clocks.apply(
    f.batch(5, 351, [
      { kind: "patch", id: f.patch.id, source: f.patch.source, phase: "initial", tick: 0 },
    ]),
  );
  assert.equal(clocks.physicalTick("rock"), undefined);
});

test("event batches validate atomically and reject duplicate, stale, or foreign owners", () => {
  const f = fixture(),
    clocks = new StateEventClocks(f.contract);
  const sample: StateClockSample = {
    kind: "element",
    id: f.body.id,
    source: f.body.source,
    active: true,
    tick: 2,
  };
  const bad: StateClockSample = { ...sample, source: { ...sample.source, index: 999 } };
  assert.throws(() => clocks.apply(f.batch(0, 0, [sample, { ...bad, id: "alien" }])), /Unbound/);
  assert.equal(clocks.sample("element", sample.id), undefined);
  assert.throws(() => clocks.apply(f.batch(0, 0, [sample, sample])), /Duplicate/);
  assert.throws(() => clocks.apply({ ...f.batch(0, 0, [sample]), mission: "other" }), /mission/);
  assert.throws(() => clocks.apply(f.batch(0, 0, [{ ...sample, tick: -1 }])), /tick/);
  clocks.apply(f.batch(0, 10, [sample]));
  assert.throws(() => clocks.apply(f.batch(1, 9, [])), /out-of-order/);
  assert.throws(() => clocks.apply(f.batch(0, 10, [])), /out-of-order/);
  sample.tick = 100;
  assert.equal(clocks.sample("element", sample.id)!.tick, 2);
  const copy = clocks.sample("element", sample.id)!;
  copy.tick = 200;
  assert.equal(clocks.sample("element", sample.id)!.tick, 2);
});
