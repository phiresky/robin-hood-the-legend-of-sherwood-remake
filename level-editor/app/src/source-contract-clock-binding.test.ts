import test from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import { SourceContractClockBinding } from "./source-contract-clock-binding.ts";
import {
  MissionStateLayer,
  missionStateDataHash,
  type MissionStateSource,
} from "./mission-state-layer.ts";
import { validateNativeLoopPreview } from "../../shared/src/state-delivery.ts";
import type { NativeStatePresentationContract } from "../../shared/src/native-state-presentation.ts";
import type { MissionStateContract } from "../../shared/src/mission-state.ts";

async function fixture() {
  const targets = Array.from({ length: 5 }, (_, i) => ({
    position_x: i * 30,
    position_y: 40,
    position_z: -1,
    obstacle_index: 65535,
    action: 0,
    action_position_x: i * 30 + 2,
    action_position_y: 42,
    polyline: [],
  }));
  const animations = Array.from({ length: 15 }, (_, i) => ({
    sprite: { position_x: i * 10, position_y: 0, elevation: i === 12 ? 0 : 100 },
    active: true,
    display_polyline: [],
    blit_type: 0,
  }));
  const source = {
    name: "mission",
    data: { targets },
    level: { animations, sight_obstacles: [] },
    camera: { kind: "oblique-orthographic", elevation_deg: 35 },
  } as unknown as MissionStateSource;
  const pin = { path: "frame.png", sha256: "a".repeat(64), width: 1, height: 1 };
  const frames = Array.from({ length: 32 }, () => ({
    ...pin,
    offset: [0, 0] as [number, number],
    delay: 1,
  }));
  const native: NativeStatePresentationContract = {
    version: 1,
    mission: source.name,
    mission_data_sha256: await missionStateDataHash(source.data),
    level_data_sha256: await missionStateDataHash(source.level),
    camera_elevation_deg: 35,
    scope: "map-art-and-listed-effects",
    background: pin,
    origin: [0, 0],
    elements: [],
  };
  for (const [i, a] of animations.entries())
    native.elements.push({
      id: `animation${i}`,
      source: { kind: "map-animation", index: i, sha256: await missionStateDataHash(a) },
      active: true,
      loop: true,
      frames,
      display_position: [i * 10, 0],
      sort_position: [i * 10, 0],
      display_order: 0,
      creation_order: i,
      polyline: [],
    });
  for (const [i, t] of targets.entries())
    native.elements.push({
      id: `sign${i}`,
      source: { kind: "mission-target", index: i, sha256: await missionStateDataHash(t) },
      active: true,
      loop: true,
      frames,
      display_position: [t.position_x, t.position_y],
      sort_position: [t.action_position_x, t.action_position_y],
      display_order: 40,
      creation_order: 15 + i,
      polyline: [],
    });
  const physical: MissionStateContract = {
    version: 1,
    mission: source.name,
    mission_data_sha256: native.mission_data_sha256,
    level_data_sha256: native.level_data_sha256,
    camera_elevation_deg: 35,
    targets: await Promise.all(
      targets.map(async (t, i) => ({
        id: `physical${i}`,
        target_index: i,
        target_sha256: await missionStateDataHash(t),
        representation: "physical",
        model_origin: [0, 0, 0],
        source: {
          id: "sign",
          role: "objects",
          model: "sign.glb",
          model_sha256: "b".repeat(64),
          resources: [],
        },
        actions: [0, 210, 211].map((action) => ({
          action,
          clip: "turn",
          timing: { mode: "loop", cycleTicks: 64 },
        })),
      })),
    ),
  };
  return { source, native, physical };
}
function model() {
  const root = new THREE.Group(),
    mesh = new THREE.Mesh(new THREE.BoxGeometry(), new THREE.MeshBasicMaterial());
  root.add(mesh);
  root.animations = [
    new THREE.AnimationClip("turn", -1, [
      new THREE.VectorKeyframeTrack(
        mesh.uuid + ".position",
        [0, 2 / 25, 64 / 25],
        [0, 0, 0, 2, 0, 0, 0, 0, 0],
        THREE.InterpolateDiscrete,
      ),
    ]),
  ];
  return root;
}

test("late physical attachment samples independent source cursor and never internally advances", async () => {
  const f = await fixture(),
    clocks = new SourceContractClockBinding();
  await clocks.set(f.native, f.physical, f.source);
  clocks.setAllPlaying(true);
  clocks.advance(0, 13 / 25);
  const layer = new MissionStateLayer(
    () => {},
    (e) => assert.fail(e),
    () => ({ load: async () => model(), dispose() {} }),
  );
  await layer.set(f.physical, {} as FileSystemDirectoryHandle, f.source, clocks);
  assert.equal(layer.players.size, 5);
  assert.equal(layer.players.get("physical0")!.player.tick, 13);
  assert.equal(layer.advance(10), false);
  assert.equal(layer.players.get("physical0")!.player.tick, 13);
  layer.seek("physical0", 2 ** 24 + 63);
  assert.equal(layer.players.get("physical0")!.player.tick, 63);
  assert.equal(clocks.snapshot().find((r) => r.id === "animation7")!.tick, 13);
  clocks.advance(1, 1 / 25);
  assert.equal(layer.players.get("physical0")!.player.tick, 0);
  layer.selectAction("physical0", 211);
  assert.equal(layer.players.get("physical0")!.player.tick, 0);
  layer.clear();
  clocks.advance(2, 1);
  assert.equal(layer.players.size, 0);
  layer.dispose();
  clocks.dispose();
});

test("optional physical loop rejects mismatched source, cycle and missing targets", async () => {
  const f = await fixture(),
    loop = {
      version: 1,
      scope: "controlled-native-loop-preview",
      native: f.native,
      focus_element_id: "sign0",
      physical: f.physical,
    };
  validateNativeLoopPreview(loop);
  const wrong = structuredClone(loop);
  wrong.physical.mission = "other";
  assert.throws(() => validateNativeLoopPreview(wrong), /sources differ/);
  const cycle = structuredClone(loop);
  cycle.physical.targets[0]!.actions[0]!.timing = { mode: "loop", cycleTicks: 32 };
  assert.throws(() => validateNativeLoopPreview(cycle), /cycle differs/);
  const missing = structuredClone(loop);
  missing.physical.targets.pop();
  assert.throws(() => validateNativeLoopPreview(missing), /Missing physical/);
  validateNativeLoopPreview({ ...loop, physical: undefined });
});

test("clear retires in-flight external player loads without publishing or retaining consumers", async () => {
  const f = await fixture(),
    clocks = new SourceContractClockBinding();
  await clocks.set(f.native, f.physical, f.source);
  let release!: () => void, started!: () => void;
  const gate = new Promise<void>((r) => (release = r)),
    ready = new Promise<void>((r) => (started = r));
  const layer = new MissionStateLayer(
    () => {},
    (e) => assert.fail(e),
    () => ({
      load: async () => {
        started();
        await gate;
        return model();
      },
      dispose() {},
    }),
  );
  const pending = layer.set(f.physical, {} as FileSystemDirectoryHandle, f.source, clocks);
  await ready;
  layer.clear();
  clocks.clear();
  release();
  await pending;
  assert.equal(layer.players.size, 0);
  assert.equal(layer.replacedTargets.size, 0);
  layer.dispose();
  clocks.dispose();
});
