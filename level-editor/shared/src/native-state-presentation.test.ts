import test from "node:test";
import assert from "node:assert/strict";
import {
  nativeElementBehind,
  nativeBackgroundFrames,
  nativePresentationFrame,
  nativePresentationOrder,
  validateNativeStatePresentation,
  type NativePresentationElement,
  type NativeBackgroundState,
} from "./native-state-presentation.ts";

test("retained receiver frame and final animation have distinct clocks", () => {
  const frame = element("source").frames[0]!;
  const state: NativeBackgroundState = {
    id: "patch",
    source: { kind: "mission-patch", index: 0, sha256: "a".repeat(64) },
    display_position: [0, 0],
    restore_bounds: [0, 0, 2, 2],
    definitive: false,
    initial: [],
    transition: [
      { ...frame, path: "first.png", delay: 2 },
      { ...frame, path: "last.png", delay: 1 },
    ],
    final: [
      { ...frame, path: "final-a.png", delay: 0 },
      { ...frame, path: "final-b.png", delay: 0 },
    ],
    initial_loop: false,
    final_loop: true,
  };
  const paths = (phase: "forward" | "reverse", tick: number) =>
    nativeBackgroundFrames(state, phase, tick).map((f) => f.path);
  assert.deepEqual(paths("forward", 2), ["first.png"]);
  assert.deepEqual(paths("forward", 4), ["last.png"]);
  assert.deepEqual(paths("forward", 5), ["last.png", "final-a.png"]);
  assert.deepEqual(paths("forward", 6), ["last.png", "final-b.png"]);
  assert.deepEqual(paths("reverse", 1), ["last.png"]);
  assert.deepEqual(paths("reverse", 2), ["first.png"]);
  assert.deepEqual(paths("reverse", 5), []);
});

function element(id: string, polyline: [number, number][] = []): NativePresentationElement {
  return {
    id,
    source: { kind: "mission-target", index: 0, sha256: "a".repeat(64) },
    active: true,
    frames: [
      { path: "frame.png", sha256: "b".repeat(64), width: 2, height: 2, offset: [0, 0], delay: 1 },
    ],
    loop: true,
    display_position: [0, 100],
    sort_position: [5, 5],
    display_order: 100,
    creation_order: 0,
    polyline,
  };
}
test("frame entry lasts delay+1 ticks with explicit loop and clamp", () => {
  const e = element("sign");
  e.frames = Array.from({ length: 32 }, () => ({ ...e.frames[0]! }));
  assert.equal(nativePresentationFrame(e, 1), 0);
  assert.equal(nativePresentationFrame(e, 63), 31);
  assert.equal(nativePresentationFrame(e, 64), 0);
  e.loop = false;
  assert.equal(nativePresentationFrame(e, 100), 31);
  assert.throws(() => nativePresentationFrame(e, -0.1), /tick/);
});
test("global polyline insertion uses action anchor, independently of display position", () => {
  const sign = element("sign"),
    remote = element("remote", [
      [0, 10],
      [10, 10],
    ]),
    canopy = element("canopy", [
      [0, 20],
      [10, 20],
    ]);
  remote.frames = [];
  remote.creation_order = 1;
  canopy.creation_order = 2;
  assert.deepEqual(
    nativePresentationOrder([canopy, sign, remote]).map((e) => e.id),
    ["sign", "remote", "canopy"],
  );
  sign.sort_position = [5, 15];
  assert.deepEqual(
    nativePresentationOrder([canopy, sign, remote]).map((e) => e.id),
    ["remote", "sign", "canopy"],
  );
  assert.equal(
    nativeElementBehind(
      [
        [0, 10],
        [10, 20],
      ],
      [-1, 9],
    ),
    true,
  );
  assert.equal(
    nativeElementBehind(
      [
        [0, 10],
        [10, 20],
      ],
      [11, 20],
    ),
    false,
  );
  assert.equal(
    nativeElementBehind(
      [
        [0, 10],
        [10, 20],
      ],
      [5, 15],
    ),
    false,
  );
});
test("ties are admitted only when every permutation preserves painted ordering", () => {
  const sign = element("sign"),
    a = element("a", [
      [0, 10],
      [10, 10],
    ]),
    b = element("b", [
      [0, 10],
      [10, 10],
    ]);
  a.creation_order = 1;
  b.creation_order = 2;
  b.frames = [];
  assert.deepEqual(
    nativePresentationOrder([sign, a, b])
      .filter((e) => e.frames.length)
      .map((e) => e.id),
    ["sign", "a"],
  );
  b.frames = [...a.frames];
  assert.throws(() => nativePresentationOrder([sign, a, b]), /Ambiguous/);
});
test("contracts reject escaping images, duplicate source and reversed polylines", () => {
  const c = {
    version: 1,
    mission: "S03",
    mission_data_sha256: "a".repeat(64),
    level_data_sha256: "b".repeat(64),
    camera_elevation_deg: 35,
    scope: "map-art-and-listed-effects",
    background: { path: "bg.png", sha256: "c".repeat(64), width: 5, height: 5 },
    origin: [0, 0],
    elements: [element("sign")],
  };
  validateNativeStatePresentation(c);
  c.background.path = "../bg.png";
  assert.throws(() => validateNativeStatePresentation(c), /source binding/);
  c.background.path = "bg.png";
  c.elements[0]!.polyline = [
    [10, 0],
    [0, 0],
  ];
  assert.throws(() => validateNativeStatePresentation(c), /increase/);
});

test("explicit shadow contracts reject ambiguous color, format and strength", () => {
  const contract = {
    version: 1,
    mission: "S03",
    mission_data_sha256: "a".repeat(64),
    level_data_sha256: "b".repeat(64),
    camera_elevation_deg: 35,
    scope: "map-art-and-listed-effects",
    background: { path: "background.png", sha256: "c".repeat(64), width: 2, height: 2 },
    origin: [0, 0],
    elements: [element("marker")],
  };
  const frame = contract.elements[0]!.frames[0]!;
  frame.shadow_key = { rgb: [0, 0, 255], strength_percent: 40, pixel_format: "rgb565" };
  validateNativeStatePresentation(contract);
  for (const invalid of [
    { ...frame.shadow_key, rgb: [0, 0, 256] },
    { ...frame.shadow_key, rgb: [0, 255] },
    { ...frame.shadow_key, strength_percent: -1 },
    { ...frame.shadow_key, strength_percent: 101 },
    { ...frame.shadow_key, strength_percent: 40.5 },
    { ...frame.shadow_key, pixel_format: "automatic" },
    null,
  ]) {
    const changed = structuredClone(contract);
    Object.assign(changed.elements[0]!.frames[0]!, { shadow_key: invalid });
    assert.throws(() => validateNativeStatePresentation(changed), /shadow key/);
  }
});

test("background restoration rejects overlapping regions and escaped frame bounds", () => {
  const frame = element("source").frames[0]!;
  const state: NativeBackgroundState = {
    id: "patch",
    source: { kind: "mission-patch", index: 0, sha256: "a".repeat(64) },
    display_position: [0, 0],
    restore_bounds: [0, 0, 2, 2],
    definitive: true,
    initial: [],
    transition: [frame],
    final: [],
    initial_loop: false,
    final_loop: false,
  };
  const contract = {
    version: 1,
    mission: "S03",
    mission_data_sha256: "a".repeat(64),
    level_data_sha256: "b".repeat(64),
    camera_elevation_deg: 35,
    scope: "map-art-and-listed-effects",
    background: { path: "background.png", sha256: "c".repeat(64), width: 8, height: 8 },
    origin: [0, 0],
    elements: [],
    background_states: [state],
  };
  validateNativeStatePresentation(contract);
  const other = structuredClone(state);
  other.id = "other";
  other.source.index = 1;
  contract.background_states.push(other);
  assert.throws(() => validateNativeStatePresentation(contract), /overlapping/);
  other.display_position = [2, 0];
  other.restore_bounds = [2, 0, 2, 2];
  validateNativeStatePresentation(contract);
  other.transition[0]!.offset = [1, 0];
  assert.throws(() => validateNativeStatePresentation(contract), /outside restoration/);
  assert.throws(() => nativeBackgroundFrames(state, "reverse", 0), /transition/);
});
