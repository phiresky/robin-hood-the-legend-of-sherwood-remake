import test from "node:test";
import assert from "node:assert/strict";
import { componentClipFixture } from "../test-fixtures/component-clips.ts";
import { validateStateDelivery } from "./state-delivery.ts";
import { physicalElementComponentClipPose } from "./component-clips.ts";

function logs() {
  const f = componentClipFixture();
  const base = f.contract.native.elements[0]!;
  f.contract.native.elements = [11, 7, 16].map((count, stage) => ({
    ...structuredClone(base),
    id: `stage${stage}`,
    source: { ...base.source, index: 27 + stage },
    display_order: stage,
    creation_order: stage,
    frames: Array.from({ length: count }, (_, i) => ({
      ...base.frames[0]!,
      delay: i === 0 && stage > 0 ? (stage === 1 ? 29 : 44) : 2,
    })),
  }));
  f.family.element_ids = f.contract.native.elements.map((e) => e.id);
  f.family.body_terminal_tick = 87;
  f.motion.native_passthrough_element_ids = [];
  f.component.nodes = Array.from({ length: 18 }, (_, i) => `log${i}`);
  f.component.stages = f.contract.native.elements.map((e, stage) => {
    let at = 0;
    const frame_ticks = e.frames.map((frame, i) => {
      const entry = at;
      at += frame.delay + 1;
      return (stage > 0 && i === 0) || (stage < 2 && i === e.frames.length - 1) ? null : entry;
    });
    return {
      element_id: e.id,
      source_element_sha256: e.source.sha256,
      source_frames: structuredClone(e.frames),
      first_source_frame: stage ? 1 : 0,
      clip_start_tick: stage === 0 ? 0 : stage === 1 ? 30 : 45,
      frame_ticks,
      gaps: frame_ticks.flatMap((v, frame) =>
        v === null
          ? [{ frame, source_sha256: e.frames[frame]!.sha256, reason: "component-absent" as const }]
          : [],
      ),
    };
  });
  f.component.visibility_phases = [
    { first_tick: 0, last_tick: 87, active_nodes: f.component.nodes },
  ];
  f.component.terminal = {
    stage_element_id: "stage2",
    source_frame: 15,
    clip_tick: 87,
    active_nodes: f.component.nodes,
    retired_nodes: [],
  };
  validateStateDelivery(f.contract);
  return f;
}
function pose(f: ReturnType<typeof logs>, ticks: number[], active = true) {
  return physicalElementComponentClipPose(
    f.contract,
    "trap",
    ticks.map((tick, i) => ({ element_id: `stage${i}`, active, tick })),
  );
}
test("full log clip retains every intermediate tick across three transparent source handoffs", () => {
  const f = logs();
  for (const tick of [0, 6, 26, 27, 28, 29, 30, 31, 41, 42, 43, 44, 45, 46, 86, 87, 88, 500]) {
    const p = pose(f, [tick, tick, tick]);
    assert.equal(p.component_clips.length, 1);
    assert.equal(p.component_clips[0]!.clip_tick, Math.min(87, tick));
    assert.equal(
      p.component_clips[0]!.element_id,
      tick < 30 ? "stage0" : tick < 45 ? "stage1" : "stage2",
    );
  }
  for (let tick = 87; tick >= 0; tick--)
    assert.equal(pose(f, [tick, tick, tick]).component_clips[0]!.clip_tick, tick);
  assert.equal(pose(f, [0, 0, 0], false).endpoint, "initial");
});
test("independent log clocks preserve empty gaps and reject overlapping stages", () => {
  const f = logs();
  assert.equal(pose(f, [30, 0, 0]).visible, false);
  assert.equal(pose(f, [30, 45, 0]).visible, false);
  assert.equal(pose(f, [30, 31, 0]).component_clips[0]!.clip_tick, 31);
  assert.throws(() => pose(f, [27, 30, 0]), /overlapping component owners/);
  assert.throws(() => pose(f, [30, 42, 45]), /overlapping component owners/);
});
test("log stage contracts reject missing stages, false timing and unbound transparent gaps", () => {
  for (const change of [
    (f: ReturnType<typeof logs>) => {
      f.component.stages.splice(1, 1);
    },
    (f: ReturnType<typeof logs>) => {
      f.component.stages[1]!.frame_ticks[1] = 29;
    },
    (f: ReturnType<typeof logs>) => {
      f.component.stages[0]!.gaps = [];
    },
    (f: ReturnType<typeof logs>) => {
      f.component.stages[2]!.source_frames[0]!.delay = 43;
    },
  ]) {
    const f = logs();
    change(f);
    assert.throws(() => validateStateDelivery(f.contract), /component clips/);
  }
});
