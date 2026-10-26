import test from "node:test";
import assert from "node:assert/strict";
import { componentClipFixture } from "../test-fixtures/component-clips.ts";
import {
  validateStateDelivery,
  physicalPatchPose,
  physicalFamilyEndpoints,
} from "./state-delivery.ts";
import {
  physicalElementComponentClipPose,
  nativeRemainderComponentClips,
} from "./component-clips.ts";
import { StateEventClocks } from "./state-event-clocks.ts";
test("component clips bind frame zero, exact delays and explicit native passthrough", () => {
  const f = componentClipFixture();
  validateStateDelivery(f.contract);
  assert.equal(physicalFamilyEndpoints(f.family).length, 3);
  assert.throws(() => physicalPatchPose(f.contract, "trap", 1), /independent source clocks/);
  for (const [tick, want] of [
    [0, 0],
    [1, 1],
    [2, 1],
    [3, 1],
    [4, 4],
    [100, 4],
  ])
    assert.equal(
      physicalElementComponentClipPose(f.contract, "trap", [
        { element_id: "cart", active: true, tick: tick! },
      ]).component_clips[0]!.clip_tick,
      want,
    );
  assert.deepEqual(
    physicalElementComponentClipPose(f.contract, "trap", [
      { element_id: "actors", active: true, tick: 4 },
    ]).component_clips,
    [],
  );
  assert.equal(physicalElementComponentClipPose(f.contract, "trap", []).endpoint, "initial");
  assert.equal(
    physicalElementComponentClipPose(f.contract, "trap", [
      { element_id: "actors", active: true, tick: 4 },
    ]).endpoint,
    "initial",
    "native passthrough never activates physical endpoint",
  );
});
test("component clips reject source, gap, coverage and visibility mutations before loading", () => {
  const mutations = [
    (f: ReturnType<typeof componentClipFixture>) =>
      (f.component.stages[0]!.source_element_sha256 = "c".repeat(64)),
    (f: ReturnType<typeof componentClipFixture>) =>
      (f.component.stages[0]!.source_frames[1]!.delay = 0),
    (f: ReturnType<typeof componentClipFixture>) => (f.component.stages[0]!.frame_ticks[2] = 2),
    (f: ReturnType<typeof componentClipFixture>) => (f.component.stages[0]!.frame_ticks[0] = null),
    (f: ReturnType<typeof componentClipFixture>) => (f.motion.native_passthrough_element_ids = []),
    (f: ReturnType<typeof componentClipFixture>) =>
      f.motion.native_passthrough_element_ids.push("cart"),
    (f: ReturnType<typeof componentClipFixture>) => f.component.nodes.push("old"),
    (f: ReturnType<typeof componentClipFixture>) =>
      (f.component.visibility_phases[1]!.first_tick = 2),
    (f: ReturnType<typeof componentClipFixture>) => f.component.terminal.active_nodes.push("old"),
    (f: ReturnType<typeof componentClipFixture>) =>
      ((f.component as unknown as Record<string, unknown>).unknown = true),
  ];
  for (const change of mutations) {
    const f = componentClipFixture();
    change(f);
    assert.throws(() => validateStateDelivery(f.contract), /component clips/);
  }
});
test("source-bound gaps and remainder pairing stay explicit", () => {
  const f = componentClipFixture(),
    s = f.component.stages[0]!;
  s.frame_ticks = [null, 0, 3];
  s.first_source_frame = 1;
  s.gaps = [{ frame: 0, source_sha256: s.source_frames[0]!.sha256, reason: "component-absent" }];
  f.component.visibility_phases = [{ first_tick: 0, last_tick: 3, active_nodes: ["wreck"] }];
  f.component.terminal.clip_tick = 3;
  validateStateDelivery(f.contract);
  assert.deepEqual(
    physicalElementComponentClipPose(f.contract, "trap", [
      { element_id: "cart", active: true, tick: 0 },
    ]).component_clips,
    [],
  );
  assert.deepEqual(
    nativeRemainderComponentClips(f.family, "cart", 1, {
      kind: "source-frame-components",
      element_id: "cart",
      frame: 1,
      components: ["body"],
    }),
    [{ component_id: "body", element_id: "cart", clip_tick: 0 }],
  );
  assert.throws(
    () =>
      nativeRemainderComponentClips(f.family, "cart", 0, {
        kind: "source-frame-components",
        element_id: "cart",
        frame: 0,
        components: ["body"],
      }),
    /coverage/,
  );
  s.gaps[0]!.source_sha256 = "f".repeat(64);
  assert.throws(() => validateStateDelivery(f.contract), /gap evidence/);
});
test("independent body and cask clocks retain body during cask handoff and reject overlap atomically", () => {
  const f = componentClipFixture();
  f.motion.native_passthrough_element_ids = [];
  const cask = structuredClone(f.component);
  cask.id = "cask";
  cask.nodes = ["cask"];
  cask.terminal.active_nodes = ["cask"];
  cask.terminal.retired_nodes = [];
  cask.visibility_phases = [{ first_tick: 0, last_tick: 8, active_nodes: ["cask"] }];
  cask.stages[0]!.frame_ticks = [0, 1, null];
  cask.stages[0]!.gaps = [{ frame: 2, source_sha256: "a".repeat(64), reason: "component-absent" }];
  const second = structuredClone(cask.stages[0]!);
  second.element_id = "actors";
  second.frame_ticks = [null, 5, 8];
  second.first_source_frame = 1;
  second.clip_start_tick = 5;
  second.gaps = [{ frame: 0, source_sha256: "a".repeat(64), reason: "component-absent" }];
  cask.stages.push(second);
  cask.terminal.stage_element_id = "actors";
  cask.terminal.clip_tick = 8;
  f.motion.components.push(cask);
  validateStateDelivery(f.contract);
  const clocks = new StateEventClocks(f.contract);
  const batch = (seq: number, t1: number, t2: number) => ({
    mission: "test",
    mission_data_sha256: "a".repeat(64),
    level_data_sha256: "a".repeat(64),
    sequence: seq,
    simulation_tick: seq,
    samples: f.contract.native.elements.map((e, i) => ({
      kind: "element" as const,
      id: e.id,
      source: e.source,
      active: true,
      tick: i ? t2 : t1,
    })),
  });
  clocks.apply(batch(0, 4, 1));
  assert.deepEqual(
    clocks.physicalPose("trap").component_clips?.map((p) => [p.component_id, p.clip_tick]),
    [
      ["body", 4],
      ["cask", 5],
    ],
  );
  assert.throws(() => clocks.apply(batch(1, 1, 1)), /overlapping component owners/);
  assert.equal(clocks.physicalPose("trap").component_clips?.[0]!.clip_tick, 4);
  clocks.apply(batch(2, 4, 4));
  assert.equal(clocks.physicalPose("trap").component_clips?.[1]!.clip_tick, 8);
});

test("three-owner body/wheels/cask delivery holds each completed component independently", () => {
  const f = componentClipFixture();
  f.motion.native_passthrough_element_ids = [];
  const third = structuredClone(f.contract.native.elements[1]!);
  third.id = "cask-owner";
  third.source.index = 2;
  third.display_order = third.creation_order = 2;
  f.contract.native.elements.push(third);
  f.family.element_ids.push(third.id);
  const staged = (id: string, lastOwner: string, end: number) => {
    const c = structuredClone(f.component);
    c.id = id;
    c.nodes = [id];
    c.terminal = {
      stage_element_id: lastOwner,
      source_frame: 2,
      clip_tick: end,
      active_nodes: [id],
      retired_nodes: [],
    };
    c.visibility_phases = [{ first_tick: 0, last_tick: end, active_nodes: [id] }];
    const first = c.stages[0]!;
    first.frame_ticks = [0, 1, null];
    first.gaps = [{ frame: 2, source_sha256: "a".repeat(64), reason: "component-absent" }];
    const next = structuredClone(first);
    next.element_id = "actors";
    next.first_source_frame = 1;
    next.clip_start_tick = 5;
    next.frame_ticks = [null, 5, 8];
    next.gaps = [{ frame: 0, source_sha256: "a".repeat(64), reason: "component-absent" }];
    c.stages.push(next);
    return c;
  };
  const wheels = staged("wheels", "actors", 8),
    cask = staged("cask", "cask-owner", 12);
  cask.stages[1]!.frame_ticks[2] = null;
  cask.stages[1]!.gaps.push({
    frame: 2,
    source_sha256: "a".repeat(64),
    reason: "component-absent",
  });
  const last = structuredClone(cask.stages[1]!);
  last.element_id = third.id;
  last.clip_start_tick = 9;
  last.frame_ticks = [null, 9, 12];
  last.gaps = last.gaps.slice(0, 1);
  cask.stages.push(last);
  f.motion.components.push(wheels, cask);
  validateStateDelivery(f.contract);
  const pose = (a: number, b: number, c: number) =>
    physicalElementComponentClipPose(f.contract, "trap", [
      { element_id: "cart", active: true, tick: a },
      { element_id: "actors", active: true, tick: b },
      { element_id: third.id, active: true, tick: c },
    ]);
  assert.deepEqual(
    pose(4, 4, 1).component_clips.map((s) => [s.component_id, s.clip_tick]),
    [
      ["body", 4],
      ["wheels", 8],
      ["cask", 9],
    ],
  );
  assert.deepEqual(
    pose(99, 99, 99).component_clips.map((s) => [s.component_id, s.clip_tick]),
    [
      ["body", 4],
      ["wheels", 8],
      ["cask", 12],
    ],
  );
  assert.deepEqual(
    pose(4, 4, 0).component_clips.map((s) => s.component_id),
    ["body", "wheels"],
    "cask startup gap never erases retained body/wheels",
  );
});
