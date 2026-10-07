import test from "node:test";
import assert from "node:assert/strict";
import {
  executeEffectComposition,
  type EffectCompositionBackend,
  type EffectDraw,
} from "./native-effect-composition.ts";

function fixture() {
  const draw = (id: string, stage: "background" | "ordered"): EffectDraw => ({
    element: {
      id,
      source: { kind: "map-animation", index: 0, sha256: "a".repeat(64) },
      active: true,
      frames: [],
      loop: true,
      display_position: [0, 0],
      sort_position: [0, 0],
      display_order: 0,
      creation_order: 0,
      polyline: [],
    },
    phase: 0,
    tick: 0,
    stage,
    masking: "off",
    composition: "isolated-internal-depth-source-over",
    requiresDestinationKeying: false,
  });
  const plan = {
    background: [draw("05", "background")],
    ordered: [draw("01", "ordered"), draw("02", "ordered")],
  };
  const events: string[] = [];
  const backend: EffectCompositionBackend<EffectDraw> = {
    ordinaryDepth() {
      events.push("ordinary");
    },
    supportsDestinationKeying() {
      return false;
    },
    capture() {
      events.push("capture");
      return 42;
    },
    prepare(d) {
      events.push(`prepare:${d.element.id}`);
      return d;
    },
    releasePrepared(d) {
      events.push(`release:${d.element.id}`);
    },
    staticSceneWithoutBoundEffects() {
      events.push("static");
    },
    compositeBackground(d) {
      events.push(`background:${d.element.id}`);
    },
    compositeOrdered(d) {
      events.push(`ordered:${d.element.id}`);
    },
    restore(state) {
      assert.equal(state, 42);
      events.push("restore");
    },
  };
  return { plan, events, backend };
}
test("native transaction restores background before ordinary effects and retires scratch resources", () => {
  const f = fixture();
  assert.equal(
    executeEffectComposition(true, () => f.plan, f.backend),
    "native-source-order",
  );
  assert.ok(f.events.indexOf("background:05") < f.events.indexOf("ordered:01"));
  assert.ok(f.events.indexOf("ordered:01") < f.events.indexOf("ordered:02"));
  assert.equal(f.events.at(-1), "restore");
  assert.equal(f.events.filter((x) => x.startsWith("release:")).length, 3);
});
test("partial preparation failure releases successful preparations and restores borrowed state", () => {
  const f = fixture();
  f.backend.prepare = (d) => {
    if (d.element.id === "02") throw Error("decode failed");
    return d;
  };
  assert.throws(() => executeEffectComposition(true, () => f.plan, f.backend), /decode failed/);
  assert.deepEqual(f.events, ["capture", "release:05", "release:01", "restore"]);
});
test("cleanup errors cannot skip other resource cleanup or renderer restoration", () => {
  const f = fixture();
  f.backend.releasePrepared = (d) => {
    f.events.push(`release:${d.element.id}`);
    if (d.element.id === "05") throw Error("release failed");
  };
  assert.throws(() => executeEffectComposition(true, () => f.plan, f.backend), AggregateError);
  assert.equal(f.events.filter((x) => x.startsWith("release:")).length, 3);
  assert.equal(f.events.at(-1), "restore");
});
test("unsupported destination keying fails before capture and oblique bypasses native scheduling", () => {
  const f = fixture();
  f.plan.ordered[0]!.requiresDestinationKeying = true;
  assert.throws(
    () => executeEffectComposition(true, () => f.plan, f.backend),
    /Destination keying/,
  );
  assert.deepEqual(f.events, []);
  assert.equal(
    executeEffectComposition(
      false,
      () => {
        throw Error("not native");
      },
      f.backend,
    ),
    "ordinary-depth",
  );
  assert.deepEqual(f.events, ["ordinary"]);
});
