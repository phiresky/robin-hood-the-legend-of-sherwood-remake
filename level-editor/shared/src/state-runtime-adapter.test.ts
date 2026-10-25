import test from "node:test";
import assert from "node:assert/strict";
import {
  StateRuntimeAdapter,
  type StateRuntimeBinding,
  type RuntimeStateObservation,
} from "./state-runtime-adapter.ts";
import type { StateDeliveryContract } from "./state-delivery.ts";

function fixture() {
  const source = (kind: string, index: number) => ({ kind, index, sha256: "a".repeat(64) });
  const frames = [{ delay: 2 }, { delay: 3 }];
  const owner = { id: "owner", source: source("mission-target", 17), frames };
  const patch = {
    id: "motion",
    source: source("map-patch", 6),
    initial: [],
    transition: frames,
    final: [{ delay: 0 }],
    definitive: true,
  };
  const ground = { ...patch, id: "ground", source: source("mission-patch", 3) };
  const contract = {
    native: {
      mission: "Tac06_FoB_EC",
      mission_data_sha256: "b".repeat(64),
      level_data_sha256: "c".repeat(64),
      elements: [owner],
      patch_states: [patch],
      background_states: [ground],
    },
    families: [
      {
        id: "rock",
        element_ids: ["owner"],
        patch_ids: ["motion"],
        background_ids: ["ground"],
        body_terminal_tick: 6,
        physical: { motion: { patch_id: "motion" } },
      },
    ],
  } as unknown as StateDeliveryContract;
  const bindings = [
    {
      kind: "element",
      id: "owner",
      source: owner.source,
      runtime_index: 108,
      initial_action: "WaitingUprightBored",
      initial_row: 0,
      action: "TransitionSittingWaitingUpright",
      action_row: 1,
    },
    {
      kind: "patch",
      id: "motion",
      source: patch.source,
      runtime_index: 6,
      phase_rows: { forward: 1, applied: 2 },
    },
    {
      kind: "background",
      id: "ground",
      source: ground.source,
      runtime_index: 18,
      phase_rows: { forward: 1, applied: 2 },
    },
  ] as StateRuntimeBinding[];
  const sprite = { active: true, row: 0, frame: 0, count: 0, last_action: "WaitingUprightBored" };
  const observed: RuntimeStateObservation = {
    mission: "Tac06_FoB_EC",
    engine_frame: 0,
    owners: [{ index: 108, sprite }],
    patches: [
      { index: 6, applied: false, in_transition: false, sprite: null },
      {
        index: 18,
        applied: false,
        in_transition: true,
        sprite: { ...sprite, row: 1, count: 65535 },
      },
    ],
  };
  return { contract, bindings, observed };
}

test("prepared identities keep target, collision and visual patch clocks independent", () => {
  const { contract, bindings, observed } = fixture();
  const adapter = new StateRuntimeAdapter(contract, bindings);
  const first = adapter.sample(observed);
  assert.equal(first.sequence, 0);
  assert.deepEqual(
    first.samples.map((s) => s.tick),
    [0, 0, 0],
  );
  observed.engine_frame = 20;
  Object.assign(observed.owners[0].sprite, {
    last_action: "TransitionSittingWaitingUpright",
    row: 1,
    frame: 1,
    count: 2,
  });
  observed.patches[1].sprite!.count = 2;
  const second = adapter.sample(observed);
  assert.deepEqual(
    second.samples.map((s) => s.tick),
    [5, 0, 2],
  );
  observed.engine_frame = 21;
  observed.owners[0].sprite.active = false;
  const third = adapter.sample(observed);
  assert.equal(third.samples[0].kind, "element");
  assert.deepEqual(third.samples[0], {
    kind: "element",
    id: "owner",
    source: bindings[0].source,
    active: true,
    visible: false,
    tick: 5,
  });
});

test("rejected identity, action, frame and phase observations do not consume a sequence", () => {
  const { contract, bindings, observed } = fixture();
  const adapter = new StateRuntimeAdapter(contract, bindings);
  for (const mutate of [
    (o: RuntimeStateObservation) => {
      o.mission = "wrong";
    },
    (o: RuntimeStateObservation) => {
      o.owners[0].index = 999;
    },
    (o: RuntimeStateObservation) => {
      o.owners.push(o.owners[0]);
    },
    (o: RuntimeStateObservation) => {
      o.owners[0].sprite.last_action = "ShootBow";
    },
    (o: RuntimeStateObservation) => {
      o.owners[0].sprite.row = 9;
    },
    (o: RuntimeStateObservation) => {
      o.patches[1].sprite!.row = 8;
    },
    (o: RuntimeStateObservation) => {
      o.patches[1].sprite!.frame = 1;
      o.patches[1].sprite!.count = 65535;
    },
    (o: RuntimeStateObservation) => {
      o.patches[1].applied = true;
    },
  ]) {
    const bad = structuredClone(observed);
    mutate(bad);
    assert.throws(() => adapter.sample(bad));
  }
  assert.equal(adapter.sample(observed).sequence, 0);
  observed.engine_frame = -1;
  assert.throws(() => adapter.sample(observed));
  observed.engine_frame = 1;
  assert.equal(adapter.sample(observed).sequence, 1);
});

test("construction requires exact source pins and complete family identities", () => {
  const { contract, bindings } = fixture();
  assert.throws(() => new StateRuntimeAdapter(contract, bindings.slice(1)), /Incomplete/);
  assert.throws(() => new StateRuntimeAdapter(contract, [...bindings, bindings[0]]), /Invalid/);
  const bad = structuredClone(bindings);
  bad[0].source.sha256 = "f".repeat(64);
  assert.throws(() => new StateRuntimeAdapter(contract, bad), /Invalid/);
});

test("final reset has a new phase clock even when engine elapsed time is large", () => {
  const { contract, bindings, observed } = fixture();
  const adapter = new StateRuntimeAdapter(contract, bindings);
  adapter.sample(observed);
  observed.engine_frame = 900;
  Object.assign(observed.patches[1], {
    applied: true,
    in_transition: false,
    sprite: { active: true, row: 2, frame: 0, count: 65535, last_action: "NonanimationEnd" },
  });
  const batch = adapter.sample(observed);
  assert.equal(batch.samples[2].tick, 0);
  assert.equal((batch.samples[2] as { phase: string }).phase, "applied");
});

test("prepared progression distinguishes a startup action already selected before freeze", () => {
  const f = fixture();
  const binding = f.bindings[0]!;
  if (binding.kind !== "element") throw Error("fixture");
  binding.initial_action = binding.action;
  binding.initial_row = binding.action_row;
  binding.progression = { initial: 0, action: 8 };
  const sprite = f.observed.owners[0]!.sprite;
  Object.assign(sprite, { row: 1, last_action: binding.action, target_progression: 0 });
  const a = new StateRuntimeAdapter(f.contract, f.bindings);
  assert.equal((a.sample(f.observed).samples[0] as { active: boolean }).active, false);
  f.observed.engine_frame = 1;
  Object.assign(sprite, { target_progression: 8, count: 65535 });
  const started = a.sample(f.observed).samples[0]!;
  assert.deepEqual([started.kind === "element" && started.active, started.tick], [true, 0]);
  f.observed.engine_frame = 2;
  Object.assign(sprite, { target_progression: 7 });
  assert.throws(() => a.sample(f.observed), /Unbound runtime target progression/);
  Object.assign(sprite, { target_progression: 8, count: 0 });
  assert.equal(a.sample(f.observed).sequence, 2, "rejection does not consume sequence");
});

test("prepared frame offsets map native action frame zero past synthetic prefixes without waiting again", () => {
  const f = fixture();
  const binding = f.bindings[0]!;
  if (binding.kind !== "element") throw Error("fixture");
  binding.action_frame_offset = 1;
  f.contract.native.elements[0]!.frames.unshift({
    ...f.contract.native.elements[0]!.frames[0]!,
    delay: 99,
  });
  const sprite = f.observed.owners[0]!.sprite;
  Object.assign(sprite, { row: 1, last_action: binding.action, frame: 0, count: 65535 });
  const a = new StateRuntimeAdapter(f.contract, f.bindings);
  assert.equal(a.sample(f.observed).samples[0]!.tick, 100);
  f.observed.engine_frame = 1;
  Object.assign(sprite, { frame: 1, count: 2 });
  assert.equal(a.sample(f.observed).samples[0]!.tick, 105);
  f.observed.engine_frame = 2;
  Object.assign(sprite, { frame: 2, count: 0 });
  assert.throws(() => a.sample(f.observed), /Invalid source sprite clock/);
  for (const offset of [-1, 0.5, 3]) {
    binding.action_frame_offset = offset;
    assert.throws(() => new StateRuntimeAdapter(f.contract, f.bindings), /frame offset/);
  }
});
