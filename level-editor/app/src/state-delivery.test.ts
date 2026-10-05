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
      return await new Promise<THREE.Object3D>((_resolve, fail) => {
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
