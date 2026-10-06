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
