import { retainPhaseMaterials } from "./state-material-frames.ts";
import test from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import { encode } from "fast-png";
import { StateDelivery } from "./state-delivery.ts";
import { missionStateDataHash, type MissionStateSource } from "./mission-state-layer.ts";
import type { StateDeliveryContract } from "../../shared/src/state-delivery.ts";

async function fixture() {
  const bytes = encode({
    width: 1,
    height: 1,
    channels: 4,
    data: new Uint8Array([200, 30, 20, 255]),
  });
  const hash = Array.from(
    new Uint8Array(await crypto.subtle.digest("SHA-256", new Uint8Array(bytes))),
    (n) => n.toString(16).padStart(2, "0"),
  ).join("");
  const target = {
    position_x: 0,
    position_y: 0,
    action_position_x: 0,
    action_position_y: 0,
    polyline: [],
  };
  const source = {
    name: "test",
    data: { targets: [target] },
    level: { animations: [], patches: [] },
    camera: { kind: "oblique-orthographic", elevation_deg: 35 },
  } as unknown as MissionStateSource;
  const frame = {
    path: "frame.png",
    sha256: hash,
    width: 1,
    height: 1,
    offset: [0, 0] as [number, number],
    delay: 2,
  };
  const asset = (id: string) => ({
    id,
    role: "objects" as const,
    model: id + ".glb",
    model_sha256: "a".repeat(64),
    resources: [],
  });
  const contract: StateDeliveryContract = {
    version: 1,
    scope: "controlled-state-preview",
    native: {
      version: 1,
      mission: "test",
      mission_data_sha256: await missionStateDataHash(source.data),
      level_data_sha256: await missionStateDataHash(source.level),
      camera_elevation_deg: 35,
      scope: "map-art-and-listed-effects",
      background: frame,
      origin: [0, 0],
      elements: [
        {
          id: "body",
          source: { kind: "mission-target", index: 0, sha256: await missionStateDataHash(target) },
          active: false,
          frames: [frame, frame],
          loop: false,
          display_position: [0, 0],
          sort_position: [0, 0],
          display_order: 0,
          creation_order: 0,
          polyline: [],
        },
      ],
    },
    families: [
      {
        id: "trap",
        element_ids: ["body"],
        background_ids: [],
        body_terminal_tick: 3,
        physical: { initial: [asset("initial")], applied: [asset("applied")] },
      },
    ],
  };
  const template = new THREE.Group();
  template.add(new THREE.Mesh(new THREE.BoxGeometry(), new THREE.MeshBasicMaterial()));
  return {
    source,
    contract,
    template,
    read: async () => new Uint8Array(bytes),
    library: {} as FileSystemDirectoryHandle,
  };
}
test("native timing and physical endpoint selection stay independent; clones and disposal are isolated", async () => {
  const f = await fixture();
  let disposals = 0;
  const loader = () => ({
    load: async () => f.template,
    dispose: () => {
      disposals++;
    },
  });
  const a = new StateDelivery(loader),
    b = new StateDelivery(loader);
  await Promise.all([
    a.set(f.contract, f.source, f.library, f.read),
    b.set(f.contract, f.source, f.library, f.read),
  ]);
  assert.notEqual(a.physical.children[0]!.children[0], b.physical.children[0]!.children[0]);
  a.physical.children[0]!.position.x = 17;
  assert.equal(b.physical.children[0]!.position.x, 0);
  a.activate("trap");
  a.setPlaying(true);
  a.advance(4 / 25);
  assert.equal(a.familyTick("trap"), 4);
  assert.equal(b.familyTick("trap"), undefined);
  a.selectMode("physical-endpoint");
  a.selectEndpoint("trap", "applied");
  assert.equal(a.physical.children[1]!.visible, true);
  a.advance(1);
  assert.equal(a.familyTick("trap"), 4);
  assert.throws(() => a.setPlaying(true), /no transition playback/);
  a.selectMode("native-art");
  assert.equal(a.familyTick("trap"), 4);
  a.reset("trap");
  assert.equal(a.familyTick("trap"), undefined);
  assert.equal(a.physical.visible, false);
  a.dispose();
  a.dispose();
  b.dispose();
  assert.equal(disposals, 2);
});
test("failed or stale physical loads never adopt a partial delivery", async () => {
  const f = await fixture();
  let reject!: (error: Error) => void, started!: () => void;
  const began = new Promise<void>((resolve) => {
    started = resolve;
  });
  let disposed = 0;
  const layer = new StateDelivery(() => ({
    load: async () => {
      started();
      return await new Promise<THREE.Group>((_resolve, fail) => {
        reject = fail;
      });
    },
    dispose: () => {
      disposed++;
    },
  }));
  const loading = layer.set(f.contract, f.source, f.library, f.read);
  await began;
  layer.clear();
  reject(new Error("late missing resource"));
  assert.equal(await loading, false);
  assert.equal(layer.ready, false);
  assert.equal(layer.physical.children.length, 0);
  assert.equal(disposed, 1);
  const broken = new StateDelivery(() => ({
    load: async () => {
      throw new Error("hash mismatch");
    },
    dispose: () => {},
  }));
  await assert.rejects(broken.set(f.contract, f.source, f.library, f.read), /hash mismatch/);
  assert.equal(broken.ready, false);
  assert.equal(broken.native.ready, false);
  const unavailable = new StateDelivery(() => {
    throw new Error("library unavailable");
  });
  await assert.rejects(
    unavailable.set(f.contract, f.source, f.library, f.read),
    /library unavailable/,
  );
  assert.equal(unavailable.ready, false);
  assert.equal(unavailable.native.ready, false);
  unavailable.dispose();
  layer.dispose();
  broken.dispose();
});

test("reset restores the separately pinned stationary target artwork", async () => {
  const f = await fixture();
  const bytes = encode({
    width: 1,
    height: 1,
    channels: 4,
    data: new Uint8Array([10, 90, 220, 255]),
  });
  const hash = Array.from(
    new Uint8Array(await crypto.subtle.digest("SHA-256", new Uint8Array(bytes))),
    (n) => n.toString(16).padStart(2, "0"),
  ).join("");
  f.contract.native.elements[0]!.initial_frame = {
    ...f.contract.native.elements[0]!.frames[0]!,
    path: "initial.png",
    sha256: hash,
  };
  const player = new StateDelivery(() => ({ load: async () => f.template, dispose() {} }));
  await player.set(f.contract, f.source, f.library, async (resource) =>
    resource.path === "initial.png" ? new Uint8Array(bytes) : f.read(),
  );
  assert.deepEqual(Array.from(player.native.pixels().data), [10, 90, 220, 255]);
  player.activate("trap");
  assert.deepEqual(Array.from(player.native.pixels().data), [200, 30, 20, 255]);
  player.reset("trap");
  assert.deepEqual(Array.from(player.native.pixels().data), [10, 90, 220, 255]);
  player.dispose();
});

test("mission translation preserves the reusable root transform and independent endpoint bounds", async () => {
  const f = await fixture();
  f.template.position.set(1, 2, 3);
  f.template.rotation.y = 0.4;
  f.template.scale.set(2, 1, 3);
  const sources = f.contract.families[0]!.physical.initial;
  assert.ok(Array.isArray(sources));
  sources[0]!.position = [10, 20, 30];
  const player = new StateDelivery(() => ({ load: async () => f.template, dispose() {} }));
  await player.set(f.contract, f.source, f.library, f.read);
  const initial = player.physical.children[0]!.children[0]!,
    applied = player.physical.children[1]!.children[0]!;
  assert.deepEqual(initial.position.toArray(), [11, 22, 33]);
  assert.deepEqual(applied.position.toArray(), [1, 2, 3]);
  assert.deepEqual(initial.quaternion.toArray(), f.template.quaternion.toArray());
  assert.deepEqual(initial.scale.toArray(), f.template.scale.toArray());
  const a = new THREE.Box3().setFromObject(initial),
    b = new THREE.Box3().setFromObject(applied);
  for (const key of ["min", "max"] as const)
    assert.ok(
      a[key]
        .clone()
        .sub(b[key])
        .distanceTo(new THREE.Vector3(10, 20, 30)) < 1e-10,
    );
  initial.position.x = 99;
  assert.equal(applied.position.x, 1);
  assert.equal(f.template.position.x, 1);
  player.dispose();
});

test("absent initial state loads no placeholder and switching to the reviewed final state is explicit", async () => {
  const f = await fixture();
  f.contract.families[0]!.physical.initial = { kind: "absent" };
  const calls: string[] = [];
  const player = new StateDelivery(() => ({
    load: async (binding) => {
      calls.push(binding.id);
      return f.template;
    },
    dispose() {},
  }));
  await player.set(f.contract, f.source, f.library, f.read);
  assert.deepEqual(calls, ["applied"]);
  assert.equal(player.physical.children[0]!.children.length, 0);
  player.selectMode("physical-endpoint");
  assert.equal(player.physical.children[1]!.visible, false);
  player.selectEndpoint("trap", "applied");
  assert.equal(player.physical.children[1]!.visible, true);
  assert.equal(player.physical.children[1]!.children.length, 1);
  player.clear();
  assert.equal(player.physical.children.length, 0);
});

test("nonintegrating branch hides initial rig, advances one clock and restores it on reset", async () => {
  const f = await fixture(),
    e = f.contract.native.elements[0]!,
    initial = e.frames[0]!;
  e.initial_frame = initial;
  e.frames = [];
  const profile = { name: "net", center_x: 0, center_y: 0 },
    profileBytes = new TextEncoder().encode(JSON.stringify(profile));
  const make = async (path: string, color: number[]) => {
    const bytes = encode({ width: 1, height: 1, channels: 4, data: new Uint8Array(color) });
    const digest = Array.from(
      new Uint8Array(await crypto.subtle.digest("SHA-256", new Uint8Array(bytes))),
      (n) => n.toString(16).padStart(2, "0"),
    ).join("");
    return {
      bytes,
      frame: {
        path,
        sha256: digest,
        width: 1,
        height: 1,
        offset: [0, 0] as [number, number],
        delay: 1,
      },
    };
  };
  const blue = await make("blue.png", [0, 0, 200, 255]),
    green = await make("green.png", [0, 200, 0, 255]);
  f.contract.native.background = blue.frame;
  const row = {
    integrate_in_background: false,
    definitive: false,
    start_animation_valid: false,
    transition_animation_valid: true,
    end_animation_valid: false,
    element_fx: {
      sprite: { position_x: 0, position_y: 0, elevation: 1, profile_name: "net" },
      active: true,
      display_polyline: [],
    },
  };
  f.source.data.mission_patches = [row];
  f.contract.native.mission_data_sha256 = await missionStateDataHash(f.source.data);
  f.contract.native.patch_states = [
    {
      id: "bag",
      source: { kind: "mission-patch", index: 0, sha256: await missionStateDataHash(row) },
      profile: {
        path: "profile.json",
        sha256: await missionStateDataHash(profile),
        name: "net",
        center: [0, 0],
      },
      integrate_in_background: false,
      elevation: 1,
      layer: "ordered",
      display_position: [0, 0],
      sort_position: [0, 0],
      display_order: 1,
      creation_order: 1,
      polyline: [],
      definitive: false,
      initial: [],
      transition: [green.frame],
      final: [],
      initial_loop: true,
      final_loop: true,
    },
  ];
  const family = f.contract.families[0]!;
  family.element_ids = [];
  family.hidden_initial_element_ids = [e.id];
  family.patch_ids = ["bag"];
  family.body_terminal_tick = 1;
  const p = new StateDelivery(() => ({ load: async () => f.template, dispose() {} }));
  await p.set(f.contract, f.source, f.library, async (resource) =>
    resource.path === "profile.json"
      ? profileBytes
      : resource.path === "blue.png"
        ? blue.bytes
        : resource.path === "green.png"
          ? green.bytes
          : f.read(),
  );
  const rgb = () => Array.from(p.native.pixels().data.slice(0, 3));
  assert.deepEqual(rgb(), [200, 30, 20]);
  p.activate("trap");
  assert.deepEqual(rgb(), [0, 200, 0]);
  p.setPlaying(true);
  p.advance(1 / 25);
  assert.deepEqual(rgb(), [0, 0, 200]);
  p.reset("trap");
  assert.deepEqual(rgb(), [200, 30, 20]);
  p.clear();
  assert.equal(p.ready, false);
  assert.equal(p.native.ready, false);
});

async function motionFixture() {
  const f = await fixture();
  const row = {
    integrate_in_background: false,
    definitive: true,
    start_animation_valid: false,
    transition_animation_valid: true,
    end_animation_valid: false,
    element_fx: {
      sprite: { position_x: 0, position_y: 0, elevation: 1, profile_name: "gate" },
      active: true,
      display_polyline: [],
    },
  };
  const profile = { name: "gate", center_x: 0, center_y: 0 };
  const profileBytes = new TextEncoder().encode(JSON.stringify(profile));
  f.source.data.mission_patches = [row];
  f.contract.native.mission_data_sha256 = await missionStateDataHash(f.source.data);
  f.contract.native.elements = [];
  f.contract.native.patch_states = [
    {
      id: "gate",
      source: { kind: "mission-patch", index: 0, sha256: await missionStateDataHash(row) },
      profile: {
        path: "profile.json",
        sha256: await missionStateDataHash(profile),
        name: "gate",
        center: [0, 0],
      },
      integrate_in_background: false,
      elevation: 1,
      layer: "ordered",
      display_position: [0, 0],
      sort_position: [0, 0],
      display_order: 1,
      creation_order: 1,
      polyline: [],
      definitive: true,
      initial: [],
      transition: Array.from({ length: 45 }, () => ({ ...f.contract.native.background, delay: 1 })),
      final: [],
      initial_loop: true,
      final_loop: true,
    },
  ];
  const family = f.contract.families[0]!;
  family.element_ids = [];
  family.patch_ids = ["gate"];
  family.body_terminal_tick = 89;
  family.physical.motion = {
    kind: "native-patch-translation",
    patch_id: "gate",
    source_motion_sha256: "b".repeat(64),
    translation_frames: Array.from({ length: 45 }, (_, i) => [0, i, 0]),
  };
  for (const endpoint of [family.physical.initial, family.physical.applied]) {
    assert.ok(Array.isArray(endpoint));
    endpoint[0]!.position = [20, 30, 40];
  }
  return {
    ...f,
    read: async (resource: { path: string }) =>
      resource.path === "profile.json" ? profileBytes : f.read(),
  };
}

test("reviewed physical motion follows all native frame boundaries, terminal pose, reset and disposal", async () => {
  const f = await motionFixture();
  f.template.position.set(2, 3, 4);
  f.template.rotation.y = 0.7;
  f.template.scale.set(2, 3, 4);
  const p = new StateDelivery(() => ({ load: async () => f.template, dispose() {} }));
  await p.set(f.contract, f.source, f.library, f.read);
  p.selectMode("physical-motion");
  const [covered, raised] = p.physical.children;
  const local = covered!.children[0]!;
  const matrix = local.matrix.clone();
  assert.deepEqual(local.position.toArray(), [22, 33, 44]);
  assert.equal(p.familyTick("trap"), undefined);
  p.activate("trap");
  p.setPlaying(true);
  for (let tick = 0; tick < 92; tick++) {
    assert.equal(p.familyTick("trap"), tick);
    assert.equal(covered!.visible, tick < 89);
    assert.equal(raised!.visible, tick >= 89);
    assert.equal(covered!.position.y, tick < 89 ? Math.floor(tick / 2) : 0);
    assert.deepEqual(raised!.position.toArray(), [0, 0, 0]);
    assert.deepEqual(local.position.toArray(), [22, 33, 44]);
    assert.equal(local.rotation.y, 0.7);
    assert.deepEqual(local.scale.toArray(), [2, 3, 4]);
    assert.deepEqual(local.matrix, matrix);
    p.advance(1 / 25);
  }
  p.reset("trap");
  assert.equal(covered!.visible, true);
  assert.equal(raised!.visible, false);
  assert.deepEqual(covered!.position.toArray(), [0, 0, 0]);
  p.seekFamily("trap", 76);
  assert.equal(covered!.position.y, 38);
  p.seekFamily("trap", 0);
  assert.equal(covered!.position.y, 0);
  p.selectMode("physical-endpoint");
  p.selectEndpoint("trap", "applied");
  assert.throws(() => p.setPlaying(true), /no transition playback/);
  assert.deepEqual(raised!.position.toArray(), [0, 0, 0]);
  p.dispose();
  assert.equal(p.physical.children.length, 0);
});

test("motion rejects missing review bindings and invalid pose tables before loading", async () => {
  const f = await fixture();
  const p = new StateDelivery(() => ({ load: async () => f.template, dispose() {} }));
  await p.set(f.contract, f.source, f.library, f.read);
  assert.throws(() => p.selectMode("physical-motion"), /no reviewed/);
  p.dispose();
  const m = await motionFixture();
  const motion = m.contract.families[0]!.physical.motion!;
  assert.equal(motion.kind, "native-patch-translation");
  if (motion.kind !== "native-patch-translation") throw new Error("Fixture motion mismatch");
  motion.translation_frames.pop();
  const invalid = new StateDelivery(() => {
    throw new Error("must not load");
  });
  await assert.rejects(invalid.set(m.contract, m.source, m.library, m.read), /invalid reviewed/);
  invalid.dispose();
});

test("appearing mechanism follows all45 native poses and restores baked nodes on reset and mode change", async () => {
  const f = await motionFixture();
  const family = f.contract.families[0]!;
  family.physical.initial = { kind: "absent" };
  const rotations = Array.from({ length: 45 }, (_, frame) =>
    frame === 44 ? new THREE.Matrix4() : new THREE.Matrix4().makeRotationZ((44 - frame) / 10),
  );
  family.physical.motion = {
    kind: "native-patch-node-poses",
    patch_id: "gate",
    source_motion_sha256: "c".repeat(64),
    node_frames: rotations.map((matrix) => [{ node: "link", matrix: matrix.toArray() }]),
  };
  const original = f.template.children[0]!;
  original.name = "link";
  original.position.set(2, 3, 4);
  original.updateMatrix();
  const base = original.matrix.clone();
  const p = new StateDelivery(() => ({ load: async () => f.template, dispose() {} }));
  await p.set(f.contract, f.source, {} as FileSystemDirectoryHandle, f.read);
  p.selectMode("physical-motion");
  const [initial, applied] = p.physical.children;
  const link = applied!.getObjectByName("link")!;
  assert.equal(initial!.children.length, 0);
  assert.equal(applied!.visible, false);
  p.activate("trap");
  p.setPlaying(true);
  for (let tick = 0; tick < 92; tick++) {
    if (tick) p.advance(1 / 25);
    assert.equal(p.familyTick("trap"), tick);
    const frame = Math.min(44, Math.floor(tick / 2));
    assert.equal(applied!.visible, true);
    assert.deepEqual(link.matrix.toArray(), rotations[frame]!.clone().multiply(base).toArray());
    assert.deepEqual(original.matrix.toArray(), base.toArray());
  }
  p.seekFamily("trap", 6);
  p.selectMode("physical-endpoint");
  assert.deepEqual(link.matrix.toArray(), base.toArray());
  p.selectMode("physical-motion");
  assert.deepEqual(link.matrix.toArray(), rotations[3]!.clone().multiply(base).toArray());
  p.reset("trap");
  assert.equal(applied!.visible, false);
  assert.deepEqual(link.matrix.toArray(), base.toArray());
  assert.equal(link.matrixAutoUpdate, true);
  p.dispose();
  assert.equal(p.physical.children.length, 0);
});

test("appearing mechanisms reject an unbaked terminal pose before loading", async () => {
  const f = await motionFixture();
  const family = f.contract.families[0]!;
  family.physical.initial = { kind: "absent" };
  family.physical.motion = {
    kind: "native-patch-node-poses",
    patch_id: "gate",
    source_motion_sha256: "c".repeat(64),
    node_frames: Array.from({ length: 45 }, () => [
      { node: "link", matrix: new THREE.Matrix4().makeTranslation(0, 1, 0).toArray() },
    ]),
  };
  let loaded = false;
  const p = new StateDelivery(() => ({
    load: async () => {
      loaded = true;
      return f.template;
    },
    dispose() {},
  }));
  await assert.rejects(
    p.set(f.contract, f.source, {} as FileSystemDirectoryHandle, f.read),
    /terminal node pose/,
  );
  assert.equal(loaded, false);
  p.dispose();
});

async function discreteFixture() {
  const f = await motionFixture();
  const family = f.contract.families[0]!;
  f.contract.native.patch_states![0]!.transition.length = 3;
  family.body_terminal_tick = 5;
  assert.ok(Array.isArray(family.physical.applied));
  const applied = family.physical.applied[0]!;
  family.physical.motion = {
    kind: "native-patch-model-frames",
    patch_id: "gate",
    source_motion_sha256: "c".repeat(64),
    model_frames: [
      [{ ...applied, id: "transition-0", model: "transition-0.glb" }],
      [{ ...applied, id: "transition-1", model: "transition-1.glb" }],
      [structuredClone(applied)],
    ],
  };
  return f;
}

test("discrete saved models retain all four poses on native timing without interpolation", async () => {
  const f = await discreteFixture();
  const loaded: string[] = [];
  let disposed = 0;
  f.template.position.set(2, 3, 4);
  const p = new StateDelivery(() => ({
    load: async (binding) => {
      loaded.push(binding.model);
      return f.template;
    },
    dispose() {
      disposed++;
    },
  }));
  await p.set(f.contract, f.source, f.library, f.read);
  assert.equal(loaded.length, 5);
  const groups = p.physical.children;
  const visible = () => groups.map((g) => g.visible);
  p.selectMode("physical-motion");
  assert.deepEqual(visible(), [true, false, false, false, false]);
  p.activate("trap");
  p.setPlaying(true);
  for (let tick = 0; tick < 9; tick++) {
    const selected = tick >= 5 ? 1 : 2 + Math.floor(tick / 2);
    assert.deepEqual(
      visible(),
      groups.map((_, i) => i === selected),
    );
    for (const g of groups) assert.deepEqual(g.children[0]!.position.toArray(), [22, 33, 44]);
    assert.equal(p.motionEndpoint, tick >= 5 ? "applied" : "initial");
    p.advance(1 / 25);
  }
  p.seekFamily("trap", 2);
  assert.deepEqual(visible(), [false, false, false, true, false]);
  p.selectMode("physical-endpoint");
  assert.deepEqual(visible(), [true, false, false, false, false]);
  p.selectMode("physical-motion");
  assert.deepEqual(visible(), [false, false, false, true, false]);
  p.reset("trap");
  assert.deepEqual(visible(), [true, false, false, false, false]);
  p.dispose();
  assert.equal(disposed, 1);
  assert.equal(p.physical.children.length, 0);
});

test("discrete model frames reject missing phases and unsafe transition bindings before loading", async () => {
  for (const fault of ["count", "empty", "path", "hash", "position"] as const) {
    const f = await discreteFixture();
    const motion = f.contract.families[0]!.physical.motion!;
    assert.equal(motion.kind, "native-patch-model-frames");
    if (motion.kind !== "native-patch-model-frames") throw new Error("Fixture mismatch");
    const binding = motion.model_frames[0]![0]!;
    if (fault === "count") motion.model_frames.pop();
    if (fault === "empty") motion.model_frames[0] = [];
    if (fault === "path") binding.model = "../outside.glb";
    if (fault === "hash") binding.model_sha256 = "wrong";
    if (fault === "position") binding.position = [0, NaN, 0];
    const p = new StateDelivery(() => {
      throw new Error("must not load");
    });
    await assert.rejects(
      p.set(f.contract, f.source, f.library, f.read),
      /invalid (reviewed patch model frames|physical source)/,
    );
    p.dispose();
  }
});

test("discrete terminal model cannot drift from retained applied appearance or placement", async () => {
  for (const fault of [
    "model",
    "hash",
    "scene",
    "role",
    "position",
    "resource-path",
    "resource-hash",
    "descriptor",
    "descriptor-hash",
    "final-animation",
  ] as const) {
    const f = await discreteFixture();
    const family = f.contract.families[0]!;
    const motion = family.physical.motion!;
    if (motion.kind !== "native-patch-model-frames") throw new Error("Fixture mismatch");
    const binding = motion.model_frames.at(-1)![0]!;
    assert.ok(Array.isArray(family.physical.applied));
    const applied = family.physical.applied[0]!;
    binding.resources = [{ path: "texture.png", sha256: "a".repeat(64) }];
    applied.resources = structuredClone(binding.resources);
    if (fault === "model") binding.model = "different.glb";
    if (fault === "hash") binding.model_sha256 = "d".repeat(64);
    if (fault === "scene") binding.model_scene = "different";
    if (fault === "role") binding.role = "ground";
    if (fault === "position") binding.position = [21, 30, 40];
    if (fault === "resource-path") binding.resources[0]!.path = "different.png";
    if (fault === "resource-hash") binding.resources[0]!.sha256 = "d".repeat(64);
    if (fault === "descriptor") binding.descriptor = "different.json";
    if (fault === "descriptor-hash") binding.descriptor_sha256 = "d".repeat(64);
    if (fault === "final-animation")
      f.contract.native.patch_states![0]!.final = [f.contract.native.background];
    const p = new StateDelivery(() => {
      throw new Error("must not load");
    });
    await assert.rejects(
      p.set(f.contract, f.source, f.library, f.read),
      fault === "final-animation"
        ? /do not support a final animation/
        : /terminal model frame differs/,
    );
    p.dispose();
  }
});

test("source events independently sample reviewed physical motion, reject stale bindings, and preserve endpoint mode", async () => {
  const f = await motionFixture();
  const p = new StateDelivery(() => ({ load: async () => f.template, dispose() {} }));
  await p.set(f.contract, f.source, f.library, f.read);
  p.selectMode("physical-motion");
  const patch = f.contract.native.patch_states![0]!;
  const batch = (sequence: number, phase: "initial" | "forward" | "applied", tick: number) => ({
    mission: f.contract.native.mission,
    mission_data_sha256: f.contract.native.mission_data_sha256,
    level_data_sha256: f.contract.native.level_data_sha256,
    sequence,
    simulation_tick: sequence * 20,
    samples: [{ kind: "patch" as const, id: patch.id, source: patch.source, phase, tick }],
  });
  p.sampleSourceState(batch(0, "forward", 12));
  assert.equal(p.familyTick("trap"), 12);
  assert.equal(p.physical.children[0]!.position.y, 6);
  assert.equal(p.advance(100), false);
  assert.equal(p.familyTick("trap"), 12);
  assert.throws(() => p.activate("trap"), /Source events own/);
  assert.throws(() => p.reset("trap"), /Source events own/);
  assert.throws(() => p.setPlaying(true), /Source events own/);
  assert.throws(() => p.sampleSourceState(batch(0, "forward", 30)), /out-of-order/);
  const bad = batch(1, "forward", 30);
  bad.samples[0]!.source = { ...patch.source, sha256: "0".repeat(64) };
  assert.throws(() => p.sampleSourceState(bad), /Unbound/);
  assert.equal(p.physical.children[0]!.position.y, 6);
  p.sampleSourceState(batch(1, "applied", 0));
  assert.equal(p.familyTick("trap"), 89);
  assert.equal(p.physical.children[1]!.visible, true);
  p.sampleSourceState(batch(2, "applied", 3));
  assert.equal(p.physical.children[1]!.visible, true);
  p.sampleSourceState(batch(3, "initial", 0));
  assert.equal(p.familyTick("trap"), undefined);
  assert.equal(p.physical.children[0]!.visible, true);
  assert.equal(p.physical.children[0]!.position.y, 0);
  p.selectMode("physical-endpoint");
  p.selectEndpoint("trap", "applied");
  assert.equal(p.physical.children[1]!.visible, true);
  await p.set(f.contract, f.source, f.library, f.read);
  p.activate("trap");
  p.setPlaying(true);
  p.advance(1 / 25);
  assert.equal(p.familyTick("trap"), 1);
  assert.throws(() => p.sampleSourceState(batch(4, "forward", 0)), /Reload preview/);
  p.dispose();
});

test("a foreign native clock owner rejects before consuming delivery sequence or samples", async () => {
  const f = await fixture();
  const p = new StateDelivery(() => ({ load: async () => f.template, dispose() {} }));
  await p.set(f.contract, f.source, f.library, f.read);
  const element = f.contract.native.elements[0]!;
  const batch = (sequence: number, tick: number) => ({
    mission: f.contract.native.mission,
    mission_data_sha256: f.contract.native.mission_data_sha256,
    level_data_sha256: f.contract.native.level_data_sha256,
    sequence,
    simulation_tick: tick,
    samples: [
      { kind: "element" as const, id: element.id, source: element.source, active: true, tick },
    ],
  });
  p.sampleSourceState(batch(0, 0));
  p.native.sampleExternalClocks(
    [
      {
        id: element.id,
        source: element.source,
        tick: 17,
        active: false,
        playing: false,
      },
    ],
    element.id,
  );
  assert.throws(() => p.sampleSourceState(batch(1, 3)), /already owns/);
  assert.equal(p.native.tick, 17);
  // Retire only the foreign native owner: the delivery ledger must still accept sequence1.
  await p.native.set(f.contract.native, f.source, f.read);
  p.sampleSourceState(batch(1, 3));
  assert.equal(p.native.tick, 0);
  p.selectMode("physical-endpoint");
  p.selectEndpoint("trap", "initial");
  p.sampleSourceState(batch(2, 20));
  assert.equal(
    p.physical.children[0]!.visible,
    true,
    "endpoint-only models remain explicitly selected",
  );
  assert.equal(p.physical.children[1]!.visible, false);
  p.dispose();
});

test("reviewed target node poses follow owner samples instead of elapsed preview time", async () => {
  const f = await fixture();
  const family = f.contract.families[0]!;
  const link = f.template.children[0]!;
  link.name = "stone";
  const pose = new THREE.Matrix4().makeTranslation(0, 7, 0).toArray();
  family.physical.motion = {
    kind: "native-element-node-poses",
    element_id: "body",
    source_motion_sha256: "c".repeat(64),
    node_frames: [
      [{ node: "stone", matrix: pose }],
      [{ node: "stone", matrix: new THREE.Matrix4().toArray() }],
    ],
  };
  const p = new StateDelivery(() => ({ load: async () => f.template, dispose() {} }));
  await p.set(f.contract, f.source, f.library, f.read);
  p.selectMode("physical-motion");
  const element = f.contract.native.elements[0]!;
  const send = (sequence: number, active: boolean, tick: number, visible = true) =>
    p.sampleSourceState({
      mission: f.contract.native.mission,
      mission_data_sha256: f.contract.native.mission_data_sha256,
      level_data_sha256: f.contract.native.level_data_sha256,
      sequence,
      simulation_tick: sequence,
      samples: [{ kind: "element", id: element.id, source: element.source, active, tick, visible }],
    });
  send(0, false, 0);
  assert.equal(p.physical.children[0]!.visible, true);
  send(1, true, 0);
  assert.equal(p.physical.children[1]!.visible, true);
  assert.deepEqual(p.physical.children[1]!.getObjectByName("stone")!.matrix.toArray(), pose);
  assert.equal(p.advance(100), false);
  assert.equal(p.familyTick("trap"), 0);
  send(2, true, 3);
  assert.deepEqual(
    p.physical.children[1]!.getObjectByName("stone")!.matrix.toArray(),
    new THREE.Matrix4().toArray(),
  );
  send(3, false, 0);
  assert.equal(p.physical.children[0]!.visible, true);
  send(4, true, 3, false);
  assert.ok(p.physical.children.every((group) => !group.visible));
  assert.equal(p.native.loopDrawSnapshot().draws.length, 0);
  send(5, true, 3, true);
  assert.equal(p.physical.children[1]!.visible, true);
  assert.equal(p.native.loopDrawSnapshot().draws.length, 1);
  p.dispose();
  const invalid = structuredClone(f.contract);
  invalid.families[0]!.physical.motion!.source_motion_sha256 = "wrong";
  const rejected = new StateDelivery(() => {
    throw new Error("must not load unreviewed target motion");
  });
  await assert.rejects(
    rejected.set(invalid, f.source, f.library, f.read),
    /invalid reviewed target motion/,
  );
  rejected.dispose();
});

async function stagedAssemblyFixture() {
  const f = await fixture();
  const sourceTarget = f.source.data.targets[0]!;
  f.source.data.targets = [
    sourceTarget,
    structuredClone(sourceTarget),
    structuredClone(sourceTarget),
  ];
  f.contract.native.mission_data_sha256 = await missionStateDataHash(f.source.data);
  const original = f.contract.native.elements[0]!,
    frame = original.frames[0]!;
  // Synthetic rig with the observed three-owner source delay/handoff layout.
  const delays = [Array(11).fill(2), [29, ...Array(6).fill(2)], [44, ...Array(15).fill(2)]];
  f.contract.native.elements = delays.map((ds, index) => ({
    ...structuredClone(original),
    id: `stage${index}`,
    source: { ...original.source, index },
    creation_order: index,
    frames: ds.map((delay) => ({ ...frame, delay })),
  }));
  const family = f.contract.families[0]!;
  family.element_ids = f.contract.native.elements.map((e) => e.id);
  family.body_terminal_tick = 87;
  f.template.children[0]!.name = "whole-assembly";
  family.physical.motion = {
    kind: "native-element-stage-poses",
    source_motion_sha256: "c".repeat(64),
    stages: delays.map((ds, stage) => ({
      element_id: `stage${stage}`,
      node_frames: ds.map((_, index) =>
        (stage === 0 && index === 10) ||
        (stage === 1 && (index === 0 || index === 6)) ||
        (stage === 2 && index === 0)
          ? null
          : [
              {
                node: "whole-assembly",
                matrix: new THREE.Matrix4()
                  .makeTranslation(stage === 2 && index === 15 ? 0 : 100 * stage + index + 1, 0, 0)
                  .toArray(),
              },
            ],
      ),
    })),
  };
  return f;
}

test("three source owners hand off one assembly without extrapolating retired clocks", async () => {
  const f = await stagedAssemblyFixture();
  const p = new StateDelivery(() => ({ load: async () => f.template, dispose() {} }));
  await p.set(f.contract, f.source, f.library, f.read);
  p.selectMode("physical-motion");
  const send = (sequence: number, ticks: number[], active = true, visible = true) =>
    p.sampleSourceState({
      mission: f.contract.native.mission,
      mission_data_sha256: f.contract.native.mission_data_sha256,
      level_data_sha256: f.contract.native.level_data_sha256,
      sequence,
      simulation_tick: sequence,
      samples: f.contract.native.elements.map((e, index) => ({
        kind: "element",
        id: e.id,
        source: e.source,
        active,
        visible,
        tick: ticks[index]!,
      })),
    });
  const x = () => p.physical.children[1]!.getObjectByName("whole-assembly")!.matrix.elements[12];
  send(0, [0, 0, 0], false);
  assert.equal(p.physical.children[0]!.visible, true);
  send(1, [0, 0, 0]);
  assert.equal(x(), 1);
  assert.equal(p.physical.children.filter((g) => g.visible).length, 1);
  send(2, [30, 30, 30]);
  assert.equal(x(), 102);
  send(3, [30, 45, 45]);
  assert.equal(x(), 202);
  send(4, [30, 45, 87]);
  assert.equal(x(), 0);
  assert.equal(p.motionEndpoint, "applied");
  assert.equal(p.familyTick("trap"), undefined, "three observed clocks have no single family tick");
  assert.equal(p.advance(100), false);
  send(5, [30, 45, 87], true, false);
  assert.ok(p.physical.children.every((g) => !g.visible));
  send(6, [30, 45, 87]);
  assert.equal(p.physical.children[1]!.visible, true);
  p.selectMode("physical-endpoint");
  p.selectEndpoint("trap", "initial");
  send(7, [30, 45, 87]);
  assert.equal(
    p.physical.children[0]!.visible,
    true,
    "ordinary endpoint selection remains explicit",
  );
  p.dispose();
});

test("stage gaps hide the assembly; overlap rejects atomically and does not poison sequence", async () => {
  const f = await stagedAssemblyFixture();
  const p = new StateDelivery(() => ({ load: async () => f.template, dispose() {} }));
  await p.set(f.contract, f.source, f.library, f.read);
  p.selectMode("physical-motion");
  const batch = (sequence: number, ticks: number[]) => ({
    mission: f.contract.native.mission,
    mission_data_sha256: f.contract.native.mission_data_sha256,
    level_data_sha256: f.contract.native.level_data_sha256,
    sequence,
    simulation_tick: sequence,
    samples: f.contract.native.elements.map((e, index) => ({
      kind: "element" as const,
      id: e.id,
      source: e.source,
      active: true,
      visible: true,
      tick: ticks[index]!,
    })),
  });
  p.sampleSourceState(batch(0, [0, 0, 0]));
  const before = p.native.loopDrawSnapshot();
  assert.throws(() => p.sampleSourceState(batch(1, [0, 30, 30])), /Overlapping/);
  assert.deepEqual(p.native.loopDrawSnapshot(), before);
  assert.equal(p.physical.children[1]!.getObjectByName("whole-assembly")!.matrix.elements[12], 1);
  p.sampleSourceState(batch(1, [30, 0, 0]));
  assert.ok(p.physical.children.every((g) => !g.visible));
  p.sampleSourceState(batch(2, [30, 30, 0]));
  assert.equal(p.physical.children[1]!.getObjectByName("whole-assembly")!.matrix.elements[12], 102);
  p.dispose();
});

test("staged rigs reject missing owners, drifting node identity and nonterminal poses before loading", async () => {
  const f = await stagedAssemblyFixture();
  for (const fault of ["owner", "frame", "node", "terminal"]) {
    const c = structuredClone(f.contract),
      motion = c.families[0]!.physical.motion!;
    if (motion.kind !== "native-element-stage-poses") throw Error("fixture");
    if (fault === "owner") motion.stages[1]!.element_id = "stage0";
    if (fault === "frame") motion.stages[1]!.node_frames.pop();
    if (fault === "node") motion.stages[1]!.node_frames[1]![0]!.node = "different-assembly";
    if (fault === "terminal") motion.stages[2]!.node_frames.at(-1)![0]!.matrix[12] = 1;
    const p = new StateDelivery(() => {
      throw Error("invalid staged rig reached loader");
    });
    await assert.rejects(
      p.set(c, f.source, f.library, f.read),
      /staged target|stage source|node identities|terminal stage/,
    );
    p.dispose();
  }
});

test("staged preview rejects shared-clock activation and seek before any presentation mutation", async () => {
  const f = await stagedAssemblyFixture();
  const p = new StateDelivery(() => ({ load: async () => f.template, dispose() {} }));
  await p.set(f.contract, f.source, f.library, f.read);
  p.selectMode("physical-motion");
  const before = p.native.loopDrawSnapshot();
  for (const action of [
    () => p.activate("trap"),
    () => p.seekFamily("trap", 30),
    () => p.setPlaying(true),
  ]) {
    assert.throws(action, /independent source clocks/);
    assert.equal(p.familyTick("trap"), undefined);
    assert.deepEqual(p.native.loopDrawSnapshot(), before);
    assert.equal(p.physical.children[0]!.visible, true);
    assert.equal(p.physical.children[1]!.visible, false);
    assert.equal(p.advance(1), false);
  }
  const { physicalPatchPose } = await import("../../shared/src/state-delivery.ts");
  assert.throws(() => physicalPatchPose(f.contract, "trap", 30), /independent source clocks/);
  // A rejected preview action must not prevent the first authentic independent sample.
  p.sampleSourceState({
    mission: f.contract.native.mission,
    mission_data_sha256: f.contract.native.mission_data_sha256,
    level_data_sha256: f.contract.native.level_data_sha256,
    sequence: 0,
    simulation_tick: 600,
    samples: f.contract.native.elements.map((e, index) => ({
      kind: "element",
      id: e.id,
      source: e.source,
      active: true,
      tick: [30, 30, 0][index]!,
    })),
  });
  assert.equal(p.physical.children[1]!.visible, true);
  assert.equal(p.physical.children[1]!.getObjectByName("whole-assembly")!.matrix.elements[12], 102);
  p.dispose();
});

test("patch clips retain independent applied clocks, reset, and reject missing clips", async () => {
  const f = await motionFixture();
  const family = f.contract.families[0]!;
  const patch = f.contract.native.patch_states![0]!;
  patch.final = Array.from({ length: 14 }, () => ({ ...f.contract.native.background, delay: 2 }));
  f.source.data.mission_patches![0]!.end_animation_valid = true;
  patch.source.sha256 = await missionStateDataHash(f.source.data.mission_patches![0]!);
  f.contract.native.mission_data_sha256 = await missionStateDataHash(f.source.data);
  assert.ok(Array.isArray(family.physical.applied));
  const finalBinding = family.physical.applied[0]!;
  family.physical.motion = {
    kind: "native-patch-clips",
    patch_id: patch.id,
    source_motion_sha256: "b".repeat(64),
    transition: {
      binding: { ...finalBinding, id: "lift", model: "lift.glb" },
      clip: "lift",
      duration_ticks: 90,
    },
    final: { binding: finalBinding, clip: "sway", duration_ticks: 42 },
  };
  // The invisible initial patch remains a real independently clocked native phase.
  f.source.data.mission_patches![0]!.start_animation_valid = true;
  patch.source.sha256 = await missionStateDataHash(f.source.data.mission_patches![0]!);
  f.contract.native.mission_data_sha256 = await missionStateDataHash(f.source.data);
  let initialBytes: Uint8Array;
  const originalRead = f.read;
  f.read = async (resource) =>
    resource.path === "initial.png" ? initialBytes : originalRead(resource);
  for (const alpha of [255, 1, 0]) {
    initialBytes = encode({
      width: 1,
      height: 1,
      channels: 4,
      data: new Uint8Array([170, 80, 20, alpha]),
    });
    const sha256 = Array.from(
      new Uint8Array(await crypto.subtle.digest("SHA-256", new Uint8Array(initialBytes))),
      (n) => n.toString(16).padStart(2, "0"),
    ).join("");
    patch.initial = [
      { path: "initial.png", sha256, width: 1, height: 1, offset: [0, 0], delay: 1 },
    ];
    family.physical.motion.transparent_initial_frames = structuredClone(patch.initial);
    if (alpha) {
      let loaderCalls = 0;
      const rejected = new StateDelivery(() => {
        loaderCalls++;
        throw Error("must not load geometry");
      });
      await assert.rejects(
        rejected.set(f.contract, f.source, f.library, f.read),
        /not transparent/,
      );
      assert.equal(loaderCalls, 0);
      assert.equal(rejected.ready, false);
      assert.equal(rejected.native.ready, false);
      rejected.dispose();
    }
  }
  const asset = (clip: string, duration: number) => {
    const root = f.template.clone(true);
    root.animations = [
      new THREE.AnimationClip(clip, -1, [
        new THREE.VectorKeyframeTrack(
          root.uuid + ".position",
          [0, duration / 25],
          [0, 0, 0, duration, 0, 0],
        ),
      ]),
    ];
    return root;
  };
  const transition = asset("lift", 90),
    final = asset("sway", 42);
  const maps = Array.from({ length: 14 }, () => new THREE.Texture());
  const finalMaterials = maps.map((map) => new THREE.MeshBasicMaterial({ map }));
  const phaseMesh = new THREE.Mesh(new THREE.PlaneGeometry(), new THREE.MeshBasicMaterial());
  phaseMesh.name = "phase-net";
  final.add(phaseMesh);
  retainPhaseMaterials(
    final,
    new Map(
      finalMaterials.map((material, i) => [
        i,
        { material, name: `phase${i}`, texture_index: i, tex_coord: 0, alpha_mode: "OPAQUE" },
      ]),
    ),
  );
  if (!("kind" in family.physical.motion.final))
    family.physical.motion.final.material_frames = maps.map((_, i) => ({
      duration_ticks: 3,
      materials: [
        {
          node: "phase-net",
          slot: 0,
          material_index: i,
          material_name: `phase${i}`,
          texture_index: i,
          tex_coord: 0,
          alpha_mode: "OPAQUE",
        },
      ],
    }));

  const p = new StateDelivery(() => ({
    load: async (binding) =>
      binding.id === "lift" ? transition : binding.id === "applied" ? final : f.template,
    dispose() {},
  }));
  await p.set(f.contract, f.source, f.library, f.read);
  p.selectMode("physical-motion");
  let sequence = 0;
  const sample = (phase: "initial" | "forward" | "applied", tick: number) =>
    p.sampleSourceState({
      mission: f.contract.native.mission,
      mission_data_sha256: f.contract.native.mission_data_sha256,
      level_data_sha256: f.contract.native.level_data_sha256,
      sequence: sequence++,
      simulation_tick: sequence,
      samples: [{ kind: "patch", id: patch.id, source: patch.source, phase, tick }],
    });
  const applied = p.physical.children[1]!;
  const lift = p.physical.children[2]!;
  const value = (group: THREE.Object3D) => group.children[0]!.children[0]!.position.x;
  sample("forward", 12);
  assert.equal(lift.visible, true);
  assert.ok(Math.abs(value(lift) - 12) < 1e-5);
  sample("applied", 7);
  assert.equal(applied.visible, true);
  assert.ok(Math.abs(value(applied) - 7) < 1e-5);
  const materialMap = () =>
    (
      applied.getObjectByName("phase-net") as THREE.Mesh<
        THREE.BufferGeometry,
        THREE.MeshBasicMaterial
      >
    ).material.map;
  assert.equal(materialMap(), maps[2]);
  sample("applied", 49);
  assert.ok(Math.abs(value(applied) - 7) < 1e-5);
  assert.equal(materialMap(), maps[2]);
  sample("applied", 41);
  assert.equal(materialMap(), maps[13]);
  sample("applied", 42);
  assert.equal(materialMap(), maps[0]);
  sample("initial", 0);
  assert.equal(p.physical.children[0]!.visible, true);
  sample("applied", 0);
  assert.equal(value(applied), 0);
  assert.equal(materialMap(), maps[0]);
  p.dispose();
  final.animations[0]!.name = "wrong";
  const bad = new StateDelivery(() => ({ load: async () => final, dispose() {} }));
  family.physical.initial = { kind: "absent" };
  await assert.rejects(bad.set(f.contract, f.source, f.library, f.read), /clip identity/);
  bad.dispose();
});

async function remainderFixture() {
  const f = await fixture();
  const png = async (width: number, channels: 1 | 4, data: number[]) => {
    const bytes = encode({ width, height: 1, channels, data: new Uint8Array(data) });
    const sha256 = Array.from(
      new Uint8Array(await crypto.subtle.digest("SHA-256", new Uint8Array(bytes))),
      (n) => n.toString(16).padStart(2, "0"),
    ).join("");
    return { bytes, sha256 };
  };
  const background = await png(3, 4, [9, 9, 9, 255, 9, 9, 9, 255, 9, 9, 9, 255]);
  const sprite = await png(3, 4, [200, 0, 0, 255, 0, 0, 200, 255, 0, 0, 0, 0]);
  const mask = await png(3, 1, [2, 1, 0]);
  const files = new Map([
    ["background.png", background],
    ["sprite.png", sprite],
    ["mask.png", mask],
  ]);
  const image = (path: string) => ({ path, sha256: files.get(path)!.sha256, width: 3, height: 1 });
  const frame = { ...image("sprite.png"), offset: [0, 0] as [number, number], delay: 2 };
  f.contract.native.background = { ...image("background.png"), offset: [0, 0], delay: 0 };
  f.contract.native.elements[0]!.frames = [frame, frame];
  const withRemainder = structuredClone(f.contract);
  withRemainder.families[0]!.native_remainder = [
    {
      element_id: "body",
      encoding: "source-domain-v1",
      drawn_values: [2],
      priority: "remainder-over-physical",
      frames: [null, { mask: image("mask.png"), physical: "applied" }],
    },
  ];
  const read = async (resource: { path: string }) => {
    const file = files.get(resource.path);
    if (!file) throw new Error("missing " + resource.path);
    return new Uint8Array(file.bytes);
  };
  const loader = () => ({ load: async () => f.template, dispose: () => {} });
  return { ...f, withRemainder, read, loader, files, png };
}

test("native remainder keeps listed source pixels over the applied model at the same source frame", async () => {
  const f = await remainderFixture();
  const delivery = new StateDelivery(f.loader);
  assert.equal(await delivery.set(f.withRemainder, f.source, f.library, f.read), true);
  const plain = new StateDelivery(f.loader);
  assert.equal(await plain.set(f.contract, f.source, f.library, f.read), true);
  delivery.activate("trap");
  plain.activate("trap");
  assert.deepEqual(delivery.native.pixels(), plain.native.pixels());
  delivery.selectMode("native-remainder");
  assert.equal(delivery.physical.visible, false);
  // Frame 0 is unbound: the whole native frame is drawn and no physical layer is requested.
  assert.equal(delivery.remainderPhysicalRoots().size, 0);
  assert.deepEqual(delivery.native.remainderPixels(new Map()).data, plain.native.pixels().data);
  delivery.native.seek(3);
  plain.native.seek(3);
  delivery.native.setElementState("body", true, 3);
  plain.native.setElementState("body", true, 3);
  const roots = delivery.remainderPhysicalRoots();
  assert.deepEqual([...roots.keys()], ["body"]);
  assert.equal(roots.get("body"), delivery.physical.children[1]);
  const layer = {
    width: 3,
    height: 1,
    data: new Uint8Array([0, 250, 0, 255, 0, 250, 0, 255, 0, 0, 0, 0]),
  };
  const hybrid = delivery.native.remainderPixels(new Map([["body", layer]]));
  assert.deepEqual(
    [...hybrid.data],
    [200, 0, 0, 255, 0, 250, 0, 255, 9, 9, 9, 255],
    "kept native pixel wins, cart pixel shows the model, transparent source keeps the receiver",
  );
  assert.deepEqual(
    [...delivery.native.pixels().data],
    [...plain.native.pixels().data],
    "native-art composition is unchanged by a remainder binding",
  );
  assert.throws(() => delivery.native.remainderPixels(new Map()), /physical layers differ/);
  assert.throws(
    () => delivery.native.remainderPixels(new Map([["body", { ...layer, width: 2 }]])),
    /physical layers differ/,
  );
  assert.throws(() => plain.selectMode("native-remainder"), /no reviewed native remainder/);
  delivery.selectMode("physical-endpoint");
  assert.throws(() => delivery.remainderPhysicalRoots(), /not selected/);
  delivery.dispose();
  plain.dispose();
});

test("native remainder masks are hash pinned grey source-domain maps matching source coverage", async () => {
  const f = await remainderFixture();
  const changed = structuredClone(f.withRemainder);
  changed.families[0]!.native_remainder![0]!.frames[1]!.mask.sha256 = "b".repeat(64);
  const a = new StateDelivery(f.loader);
  await assert.rejects(a.set(changed, f.source, f.library, f.read), /remainder mask changed/);
  assert.equal(a.ready, false);
  for (const [channels, data, pattern] of [
    [1, [2, 1, 1], /differs from source coverage/],
    [1, [0, 1, 0], /differs from source coverage/],
    [1, [4, 1, 0], /differs from source coverage/],
    [4, [2, 2, 2, 255, 1, 1, 1, 255, 0, 0, 0, 0], /Unsupported native remainder mask/],
  ] as const) {
    const mask = await f.png(3, channels, [...data]);
    f.files.set("mask.png", mask);
    const contract = structuredClone(f.withRemainder);
    contract.families[0]!.native_remainder![0]!.frames[1]!.mask.sha256 = mask.sha256;
    const layer = new StateDelivery(f.loader);
    await assert.rejects(layer.set(contract, f.source, f.library, f.read), pattern);
    assert.equal(layer.ready, false);
    assert.equal(layer.native.ready, false);
    layer.dispose();
  }
  a.dispose();
});
