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

test("an omitted visibility field retains the owner's last visibility independently of its clock", () => {
  const f = fixture(),
    clocks = new StateEventClocks(f.contract);
  clocks.apply(
    f.batch(0, 0, [
      {
        kind: "element",
        id: f.body.id,
        source: f.body.source,
        active: true,
        visible: false,
        tick: 0,
      },
    ]),
  );
  const applied = clocks.apply(
    f.batch(1, 1, [
      { kind: "element", id: f.body.id, source: f.body.source, active: true, tick: 1 },
    ]),
  );
  assert.equal(applied[0]!.kind, "element");
  assert.equal("visible" in applied[0]! && applied[0]!.visible, false);
});

test("a target clip follows its owner's source clock: native frames, then the clip, then hidden owner", () => {
  const f = fixture();
  // Owner frames 0..1 (ticks 0..3) stay native; frame 2 starts the clip at tick 4; terminal is 6.
  f.body.frames = [{ delay: 1 }, { delay: 1 }, { delay: 1 }, { delay: 1 }] as typeof f.body.frames;
  Object.assign(f.contract.families[0]!, {
    element_ids: [f.body.id],
    body_terminal_tick: 6,
    physical: {
      motion: { kind: "native-element-clip", element_id: f.body.id, start_frame: 2 },
    },
  });
  const clocks = new StateEventClocks(f.contract);
  const owner = (sequence: number, tick: number, active = true, visible?: boolean) =>
    clocks.apply(
      f.batch(sequence, sequence, [
        {
          kind: "element",
          id: f.body.id,
          source: f.body.source as StateClockSample["source"],
          active,
          tick,
          ...(visible === undefined ? {} : { visible }),
        },
      ]),
    );
  assert.equal(clocks.physicalTick("rock"), undefined);
  assert.equal(clocks.physicalPose("rock").endpoint, "initial");
  assert.equal(clocks.physicalVisible("rock"), true);
  owner(0, 3);
  assert.equal(clocks.physicalTick("rock"), 3);
  assert.equal(clocks.physicalPose("rock").native_element, true);
  assert.equal(clocks.physicalVisible("rock"), false);
  // An unrelated patch clock cannot start the clip.
  clocks.apply(
    f.batch(1, 1, [
      {
        kind: "patch",
        id: f.patch.id,
        source: f.patch.source as StateClockSample["source"],
        phase: "applied",
        tick: 50,
      },
    ]),
  );
  assert.equal(clocks.physicalVisible("rock"), false);
  owner(2, 4);
  assert.deepEqual(clocks.physicalPose("rock"), {
    endpoint: "applied",
    translation: [0, 0, 0],
    model_frame: 0,
    clip_phase: "transition",
    clip_tick: 4,
  });
  assert.equal(clocks.physicalVisible("rock"), true);
  owner(3, 5, true, false);
  assert.equal(clocks.physicalVisible("rock"), false);
  owner(4, 9, true, true);
  assert.deepEqual(clocks.physicalPose("rock"), { endpoint: "applied", translation: [0, 0, 0] });
  assert.equal(clocks.physicalVisible("rock"), true);
  owner(5, 0, false);
  assert.equal(clocks.physicalTick("rock"), undefined);
});

test("gameplay receiver holds initial obstacles while independent target starts, then applies and resets", () => {
  const f = fixture();
  Object.assign(f.contract.families[0]!, {
    background_ids: [f.ground.id],
    patch_ids: [],
    physical: { motion: { kind: "native-element-clip", element_id: f.body.id, start_frame: 1 } },
  });
  let clocks = new StateEventClocks(f.contract);
  let seq = 0;
  const target = (tick: number) =>
    clocks.apply(
      f.batch(seq++, seq, [
        { kind: "element", id: f.body.id, source: f.body.source as StateClockSample["source"], active: true, tick },
      ]),
    );
  const receiver = (phase: "initial" | "forward" | "applied" | "reverse") =>
    clocks.apply(
      f.batch(seq++, seq, [
        { kind: "background", id: f.ground.id, source: f.ground.source as StateClockSample["source"], phase, tick: 0 },
      ]),
    );
  target(0);
  assert.equal(clocks.gameplayEndpoint("rock"), undefined, "target cannot invent a receiver state");
  receiver("initial");
  assert.equal(clocks.gameplayEndpoint("rock"), "initial");
  receiver("forward");
  target(2);
  assert.equal(
    clocks.gameplayEndpoint("rock"),
    "initial",
    "visual handoff does not swap obstacles",
  );
  receiver("applied");
  assert.equal(clocks.gameplayEndpoint("rock"), "applied");
  target(0);
  assert.equal(
    clocks.gameplayEndpoint("rock"),
    "applied",
    "target reset does not reset permanent receiver",
  );
  assert.throws(() => receiver("reverse"), /Invalid source patch phase/);
  assert.equal(clocks.gameplayEndpoint("rock"), "applied");
  receiver("initial");
  assert.equal(
    clocks.gameplayEndpoint("rock"),
    "initial",
    "explicit receiver reset is authoritative",
  );
  f.ground.definitive = false;
  clocks = new StateEventClocks(f.contract);
  receiver("applied");
  receiver("reverse");
  assert.equal(
    clocks.gameplayEndpoint("rock"),
    "applied",
    "reverse retains applied obstacles until completion",
  );
  receiver("initial");
  assert.equal(clocks.gameplayEndpoint("rock"), "initial");
});
