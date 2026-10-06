import test from "node:test";
import assert from "node:assert/strict";
import { encode } from "fast-png";
import {
  compositeNativePixels,
  NativeStatePresentation,
  type NativePixels,
} from "./native-state-presentation.ts";
import { missionStateDataHash, type MissionStateSource } from "./mission-state-layer.ts";
import type { NativeStatePresentationContract } from "../../shared/src/native-state-presentation.ts";

async function fixture() {
  const files = new Map<string, Uint8Array>();
  const image = async (path: string, color: number[]) => {
    const bytes = encode({
      width: 2,
      height: 2,
      channels: 4,
      data: new Uint8Array([...color, ...color, ...color, ...color]),
    });
    files.set(path, bytes);
    const sha256 = Array.from(
      new Uint8Array(await crypto.subtle.digest("SHA-256", new Uint8Array(bytes))),
      (n) => n.toString(16).padStart(2, "0"),
    ).join("");
    return { path, sha256, width: 2, height: 2 };
  };
  const background = await image("background.png", [20, 30, 40, 255]),
    red = await image("red.png", [200, 0, 0, 255]),
    green = await image("green.png", [0, 200, 0, 255]);
  const target = {
    position_x: 0,
    position_y: 0,
    action_position_x: 5,
    action_position_y: 5,
    polyline: [],
  };
  const source = {
    name: "S03",
    data: { targets: [target] },
    level: { animations: [], sight_obstacles: [] },
    camera: { kind: "oblique-orthographic", elevation_deg: 35 },
  } as unknown as MissionStateSource;
  const contract: NativeStatePresentationContract = {
    version: 1,
    mission: source.name,
    mission_data_sha256: await missionStateDataHash(source.data),
    level_data_sha256: await missionStateDataHash(source.level),
    camera_elevation_deg: 35,
    scope: "map-art-and-listed-effects",
    background,
    origin: [0, 0],
    elements: [
      {
        id: "sign",
        source: { kind: "mission-target", index: 0, sha256: await missionStateDataHash(target) },
        active: true,
        frames: [
          { ...red, offset: [0, 0], delay: 1 },
          { ...green, offset: [0, 0], delay: 1 },
        ],
        loop: true,
        display_position: [0, 0],
        sort_position: [5, 5],
        display_order: 0,
        creation_order: 0,
        polyline: [],
      },
    ],
  };
  return {
    image,
    contract,
    source,
    files,
    read: async (r: { path: string }) => {
      const bytes = files.get(r.path);
      if (!bytes) throw new Error("Missing image");
      return bytes;
    },
  };
}
test("background transition completion, retained terminal art and reset remain independent of target timing", async () => {
  const f = await fixture();
  const patch = {
    integrate_in_background: true,
    definitive: false,
    start_animation_valid: false,
    transition_animation_valid: true,
    end_animation_valid: false,
    element_fx: { sprite: { position_x: 0, position_y: 0, elevation: 0 } },
  };
  f.source.data.mission_patches = [patch];
  f.contract.mission_data_sha256 = await missionStateDataHash(f.source.data);
  const frames = f.contract.elements[0]!.frames;
  f.contract.background_states = [
    {
      id: "receiver",
      source: { kind: "mission-patch", index: 0, sha256: await missionStateDataHash(patch) },
      display_position: [0, 0],
      restore_bounds: [0, 0, 2, 2],
      definitive: false,
      initial: [],
      transition: frames,
      final: [],
      initial_loop: false,
      final_loop: false,
    },
  ];
  const a = new NativeStatePresentation(),
    b = new NativeStatePresentation();
  await Promise.all([a.set(f.contract, f.source, f.read), b.set(f.contract, f.source, f.read)]);
  a.setElementState("sign", false);
  b.setElementState("sign", false);
  const rgb = (p: NativeStatePresentation) => Array.from(p.pixels().data.slice(0, 3));
  assert.deepEqual(rgb(a), [20, 30, 40]);
  a.setBackgroundState("receiver", "forward");
  assert.deepEqual(rgb(a), [200, 0, 0]);
  a.setPlaying(true);
  a.advance(2 / 25);
  assert.deepEqual(rgb(a), [0, 200, 0]);
  a.advance(100 / 25);
  assert.deepEqual(rgb(a), [0, 200, 0]);
  assert.deepEqual(rgb(b), [20, 30, 40]);
  a.setBackgroundState("receiver", "reverse");
  assert.deepEqual(rgb(a), [0, 200, 0]);
  a.advance(2 / 25);
  assert.deepEqual(rgb(a), [200, 0, 0]);
  a.advance(2 / 25);
  assert.deepEqual(rgb(a), [20, 30, 40]);
  a.setBackgroundState("receiver", "applied");
  a.setElementState("sign", true, 0);
  assert.deepEqual(rgb(a), [200, 0, 0], "target draws after the retained background");
  a.setElementState("sign", false);
  a.setBackgroundState("receiver", "initial");
  assert.deepEqual(rgb(a), [20, 30, 40], "reset restores the exact original background");
  const bad = structuredClone(f.contract);
  bad.background_states![0]!.definitive = true;
  await assert.rejects(a.set(bad, f.source, f.read), /semantics/);
  assert.equal(a.ready, false);
  a.dispose();
  b.dispose();
});
test("source-over clips bounds and treats alpha0 RGB as transparent", () => {
  const target: NativePixels = {
    width: 2,
    height: 1,
    data: new Uint8Array([100, 100, 100, 255, 20, 30, 40, 255]),
  };
  compositeNativePixels(
    target,
    { width: 2, height: 1, data: new Uint8Array([255, 0, 0, 128, 0, 255, 0, 0]) },
    0,
    0,
  );
  assert.deepEqual([...target.data], [178, 50, 50, 255, 20, 30, 40, 255]);
  compositeNativePixels(
    target,
    { width: 1, height: 1, data: new Uint8Array([1, 2, 3, 255]) },
    -0.1,
    0,
  );
  assert.equal(target.data[0], 178);
});
test("paused first frame, independent seeks,25Hz wrap and disposal", async () => {
  const f = await fixture(),
    player = new NativeStatePresentation();
  await player.set(f.contract, f.source, f.read);
  assert.equal(player.isPlaying, false);
  assert.equal(player.pixels().data[0], 200);
  player.setPlaying(true);
  player.advance(0.08);
  assert.equal(player.tick, 2);
  assert.equal(player.pixels().data[1], 200);
  player.advance(0.08);
  assert.equal(player.pixels().data[0], 200);
  player.seek(2, "sign");
  assert.equal(player.pixels().data[1], 200);
  assert.equal(player.isPlaying, false);
  player.dispose();
  assert.equal(player.ready, false);
  assert.throws(() => player.pixels(), /not loaded/);
  await assert.rejects(player.set(f.contract, f.source, f.read), /disposed/);
});
test("missing or changed resources never adopt partial source scenes", async () => {
  const f = await fixture(),
    player = new NativeStatePresentation();
  f.files.delete("green.png");
  await assert.rejects(player.set(f.contract, f.source, f.read), /Missing/);
  assert.equal(player.ready, false);
  f.files.set("green.png", f.files.get("red.png")!);
  await assert.rejects(player.set(f.contract, f.source, f.read), /changed/);
  assert.equal(player.ready, false);
  const shifted = structuredClone(f.contract);
  shifted.elements[0]!.sort_position = [1, 2];
  await assert.rejects(player.set(shifted, f.source, f.read), /anchors/);
});
test("mission switch retires an in-flight load and its late failure", async () => {
  const f = await fixture(),
    player = new NativeStatePresentation();
  let reject!: (e: Error) => void, started!: () => void;
  const ready = new Promise<void>((resolve) => {
    started = resolve;
  });
  const old = player.set(f.contract, f.source, async () => {
    started();
    return new Promise<Uint8Array>((_, fail) => {
      reject = fail;
    });
  });
  await ready;
  player.clear();
  await player.set(f.contract, f.source, f.read);
  reject(new Error("Late old load failed"));
  assert.equal(await old, false);
  assert.equal(player.ready, true);
  assert.equal(player.pixels().data[0], 200);
});

test("declared shadow keys darken destination with native channel quantization", () => {
  const source = { width: 1, height: 1, data: new Uint8Array([0, 0, 255, 255]) };
  const cases = [
    { format: "rgb565", percent: 40, background: [255, 255, 255], expected: [148, 150, 148] },
    { format: "rgb555", percent: 40, background: [255, 255, 255], expected: [148, 148, 148] },
    { format: "rgb565", percent: 10, background: [255, 255, 255], expected: [222, 227, 222] },
    { format: "rgb565", percent: 40, background: [231, 73, 33], expected: [132, 40, 16] },
    { format: "rgb565", percent: 100, background: [231, 73, 33], expected: [0, 0, 0] },
    { format: "rgb565", percent: 0, background: [231, 73, 33], expected: [231, 73, 33] },
  ] as const;
  for (const row of cases) {
    const target = { width: 1, height: 1, data: new Uint8Array([...row.background, 217]) };
    compositeNativePixels(target, source, 0, 0, {
      rgb: [0, 0, 255],
      strength_percent: row.percent,
      pixel_format: row.format,
    });
    assert.deepEqual([...target.data], [...row.expected, 217]);
  }
  const ordinary = { width: 1, height: 1, data: new Uint8Array([231, 73, 33, 255]) };
  compositeNativePixels(ordinary, source, 0, 0);
  assert.deepEqual([...ordinary.data], [0, 0, 255, 255]);
  assert.deepEqual([...source.data], [0, 0, 255, 255]);
});

test("shadow composition follows clipping and layer order without painting transparent keys", () => {
  const target = { width: 1, height: 1, data: new Uint8Array([255, 255, 255, 255]) };
  const shadow = {
    rgb: [0, 0, 255] as [number, number, number],
    strength_percent: 40,
    pixel_format: "rgb565" as const,
  };
  const sprite = { width: 2, height: 1, data: new Uint8Array([0, 0, 255, 0, 0, 0, 255, 255]) };
  compositeNativePixels(target, sprite, 0, 0, shadow);
  assert.deepEqual([...target.data], [255, 255, 255, 255]);
  compositeNativePixels(target, sprite, -1, 0, shadow);
  compositeNativePixels(target, sprite, -1, 0, shadow);
  assert.deepEqual([...target.data], [82, 89, 82, 255]);
});

test("per-frame shadow semantics survive loading and phase wrap; invalid shadow alpha fails atomically", async () => {
  const f = await fixture();
  const resource = await f.image("shadow.png", [0, 0, 255, 255]);
  const shadow = {
    rgb: [0, 0, 255] as [number, number, number],
    strength_percent: 40,
    pixel_format: "rgb565" as const,
  };
  f.contract.elements[0]!.frames[0] = { ...resource, offset: [0, 0], delay: 1, shadow_key: shadow };
  const player = new NativeStatePresentation();
  await player.set(f.contract, f.source, f.read);
  assert.deepEqual(Array.from(player.pixels().data.subarray(0, 4)), [8, 16, 16, 255]);
  player.seek(2);
  assert.deepEqual(Array.from(player.pixels().data.subarray(0, 4)), [0, 200, 0, 255]);
  player.seek(4);
  assert.deepEqual(Array.from(player.pixels().data.subarray(0, 4)), [8, 16, 16, 255]);
  const invalid = await f.image("invalid-shadow.png", [0, 0, 255, 128]);
  f.contract.elements[0]!.frames[0] = { ...invalid, offset: [0, 0], delay: 1, shadow_key: shadow };
  await assert.rejects(player.set(f.contract, f.source, f.read), /binary alpha/);
  assert.equal(player.ready, false);
  player.dispose();
});

async function transientFixture(layer: "ordered" | "background" = "ordered") {
  const f = await fixture(),
    elevation = layer === "ordered" ? 1 : 0;
  const profile = { name: "net", center_x: 0, center_y: 0 };
  const profileBytes = new TextEncoder().encode(JSON.stringify(profile));
  f.files.set("profile.json", profileBytes);
  const profileHash = await missionStateDataHash(profile);
  const patch = {
    integrate_in_background: false,
    definitive: false,
    start_animation_valid: false,
    transition_animation_valid: true,
    end_animation_valid: true,
    element_fx: {
      sprite: { position_x: 0, position_y: 0, elevation, profile_name: "net" },
      active: true,
      display_polyline: [],
    },
  };
  f.source.level.patches = [];
  f.source.data.mission_patches = [patch];
  f.contract.mission_data_sha256 = await missionStateDataHash(f.source.data);
  f.contract.level_data_sha256 = await missionStateDataHash(f.source.level);
  const transparent = await f.image("transparent.png", [0, 0, 0, 0]),
    blue = await f.image("blue.png", [0, 0, 200, 255]);
  f.contract.elements[0]!.frames = [f.contract.elements[0]!.frames[0]!];
  f.contract.patch_states = [
    {
      id: "bag",
      source: { kind: "mission-patch", index: 0, sha256: await missionStateDataHash(patch) },
      profile: { path: "profile.json", sha256: profileHash, name: "net", center: [0, 0] },
      integrate_in_background: false,
      elevation,
      layer,
      display_position: [0, 0],
      sort_position: [0, 0],
      display_order: elevation,
      creation_order: 1,
      polyline: [],
      definitive: false,
      initial: [],
      transition: [
        { ...(await f.image("moving.png", [0, 200, 0, 255])), offset: [0, 0], delay: 1 },
      ],
      final: [
        { ...transparent, offset: [0, 0], delay: 0 },
        { ...blue, offset: [0, 0], delay: 1 },
      ],
      initial_loop: true,
      final_loop: true,
    },
  ];
  return f;
}
test("transient patches sort foreground separately and final never stamps transition art", async () => {
  const f = await transientFixture(),
    p = new NativeStatePresentation();
  await p.set(f.contract, f.source, f.read);
  const rgb = () => Array.from(p.pixels().data.slice(0, 3));
  assert.deepEqual(rgb(), [200, 0, 0]);
  p.setPatchState("bag", "forward", 0);
  assert.deepEqual(rgb(), [0, 200, 0]);
  p.setPatchState("bag", "forward", 2);
  assert.deepEqual(rgb(), [200, 0, 0]);
  p.setPatchState("bag", "forward", 3);
  assert.deepEqual(rgb(), [0, 0, 200]);
  p.setPatchState("bag", "forward", 5);
  assert.deepEqual(rgb(), [200, 0, 0]);
  p.setPatchState("bag", "initial");
  assert.deepEqual(rgb(), [200, 0, 0]);
  const ground = await transientFixture("background"),
    q = new NativeStatePresentation();
  await q.set(ground.contract, ground.source, ground.read);
  q.setPatchState("bag", "forward", 0);
  assert.deepEqual(Array.from(q.pixels().data.slice(0, 3)), [200, 0, 0]);
});
test("patch profile, source placement and classification are exact before adoption", async () => {
  for (const mutation of ["source", "center", "layer"]) {
    const f = await transientFixture(),
      p = new NativeStatePresentation();
    if (mutation === "source") f.contract.patch_states![0]!.source.sha256 = "0".repeat(64);
    if (mutation === "center") {
      const bytes = new TextEncoder().encode(
        JSON.stringify({ name: "net", center_x: 1, center_y: 0 }),
      );
      f.files.set("profile.json", bytes);
      f.contract.patch_states![0]!.profile.sha256 = await missionStateDataHash({
        name: "net",
        center_x: 1,
        center_y: 0,
      });
    }
    if (mutation === "layer") f.contract.patch_states![0]!.layer = "background";
    await assert.rejects(p.set(f.contract, f.source, f.read));
    assert.equal(p.ready, false);
  }
});
test("stale failing profile read cannot replace the next mission", async () => {
  const old = await transientFixture(),
    next = await fixture(),
    p = new NativeStatePresentation();
  let reject!: (e: Error) => void, started!: () => void;
  const begun = new Promise<void>((r) => (started = r)),
    blocked = new Promise<Uint8Array>((_, r) => (reject = r));
  const first = p.set(old.contract, old.source, async (resource) => {
    if (resource.path === "profile.json") {
      started();
      return blocked;
    }
    return old.read(resource);
  });
  await begun;
  await p.set(next.contract, next.source, next.read);
  reject(Error("late profile failure"));
  assert.equal(await first, false);
  assert.equal(p.ready, true);
});
async function equivalentFixture() {
  const f = await fixture();
  const a = f.contract.elements[0]!;
  a.polyline = [
    [0, 0],
    [10, 0],
  ];
  a.loop = false;
  const targetA = {
    position_x: 0,
    position_y: 0,
    action_position_x: 5,
    action_position_y: 5,
    polyline: a.polyline,
  };
  const targetB = {
    position_x: 4,
    position_y: 0,
    action_position_x: 5,
    action_position_y: 5,
    polyline: a.polyline,
  };
  f.source.data.targets = [targetA, targetB];
  f.contract.mission_data_sha256 = await missionStateDataHash(f.source.data);
  a.source.sha256 = await missionStateDataHash(targetA);
  f.contract.elements.push({
    ...structuredClone(a),
    id: "other",
    source: { kind: "mission-target", index: 1, sha256: await missionStateDataHash(targetB) },
    display_position: [4, 0],
    creation_order: 1,
  });
  const elements: [string, string] = ["sign", "other"];
  f.contract.painted_order_equivalence = {
    kind: "disjoint-adjacent-source-pair",
    elements,
    contract_sha256: await missionStateDataHash({ contract: f.contract, elements }),
  };
  return f;
}
test("painted tie needs exact contract and decoded disjoint alpha for every phase", async () => {
  const f = await equivalentFixture(),
    p = new NativeStatePresentation();
  await p.set(f.contract, f.source, f.read);
  assert.equal(p.ready, true);
  const changed = structuredClone(f.contract);
  changed.elements[1]!.frames[0]!.delay++;
  await assert.rejects(p.set(changed, f.source, f.read), /ordering contract changed/);
  assert.equal(p.ready, false);
  const overlap = await equivalentFixture();
  overlap.contract.elements[1]!.frames[0]!.offset = [-4, 0];
  const { painted_order_equivalence, ...bound } = overlap.contract;
  painted_order_equivalence!.contract_sha256 = await missionStateDataHash({
    contract: bound,
    elements: painted_order_equivalence!.elements,
  });
  await assert.rejects(p.set(overlap.contract, overlap.source, overlap.read), /pair overlaps/);
  assert.equal(p.ready, false);
});
