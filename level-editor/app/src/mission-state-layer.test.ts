import test from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import type { ProtoLevel } from "@rle/shared";
import type { MissionStateContract } from "../../shared/src/mission-state.ts";
import {
  MissionStateLayer,
  missionStateDataHash,
  resolveMissionStateTargets,
} from "./mission-state-layer.ts";

async function fixture() {
  const targets = [0, 1].map((i) => ({
    position_x: 100 + i * 100,
    position_y: 200,
    position_z: -1,
    obstacle_index: i === 0 ? 0 : 65535,
    action: 0,
    action_position_x: 900,
    action_position_y: 800,
    filename: "Panneau",
    profile_name: "Panneau",
    direction: 0,
  }));
  const source = {
    name: "test",
    data: { targets },
    level: {
      sight_obstacles: [
        {
          points: [
            { x: 0, y: 0, z_top: 36 },
            { x: 1000, y: 0, z_top: 36 },
            { x: 0, y: 1000, z_top: 36 },
          ],
        },
      ],
    } as ProtoLevel,
    camera: { kind: "oblique-orthographic" as const, elevation_deg: 35 },
  };
  const contract: MissionStateContract = {
    version: 1,
    mission: "test",
    mission_data_sha256: await missionStateDataHash(source.data),
    level_data_sha256: await missionStateDataHash(source.level),
    camera_elevation_deg: 35,
    targets: await Promise.all(
      targets.map(async (t, i) => ({
        id: "target" + i,
        target_index: i,
        target_sha256: await missionStateDataHash(t),
        source: {
          id: "sign",
          role: "objects" as const,
          model: "sign.glb",
          model_sha256: "a".repeat(64),
          resources: [],
        },
        model_origin: [0, 0, 0] as [number, number, number],
        representation: "physical" as const,
        actions: [
          { action: 0, clip: "turn", timing: { mode: "loop" as const, cycleTicks: 6 } },
          { action: 210, clip: "turn", timing: { mode: "clamp" as const, terminalTick: 5 } },
        ],
      })),
    ),
  };
  return { source, contract };
}
function asset() {
  const root = new THREE.Group();
  const mesh = new THREE.Mesh(new THREE.BoxGeometry(), new THREE.MeshBasicMaterial());
  root.add(mesh);
  root.animations = [
    new THREE.AnimationClip("turn", -1, [
      new THREE.VectorKeyframeTrack(
        mesh.uuid + ".position",
        [0, 2 / 25, 5 / 25],
        [0, 0, 0, 2, 0, 0, 5, 0, 0],
        THREE.InterpolateDiscrete,
      ),
    ]),
  ];
  return { root, mesh };
}

test("mission target physical height and action sort anchor stay independent and hash-bound", async () => {
  const f = await fixture(),
    rows = await resolveMissionStateTargets(f.contract, f.source);
  assert.deepEqual(rows[0]!.physicalPositionGame, [100, 236, 36]);
  assert.deepEqual(rows[0]!.actionPosition, [900, 800]);
  assert.deepEqual(rows[1]!.physicalPositionGame, [200, 200, 0]);
  assert.ok(Math.abs(rows[0]!.position.y - 36 / Math.cos((35 * Math.PI) / 180)) < 1e-9);
  f.source.data.targets[0]!.action_position_x++;
  await assert.rejects(resolveMissionStateTargets(f.contract, f.source), /pinned contract/);
});

test("successful instances replace only verified targets, clone independently and retain explicit action timing", async () => {
  const f = await fixture(),
    models = [asset(), asset()],
    replacements: number[][] = [],
    errors: string[] = [];
  const layer = new MissionStateLayer(
    (ids) => replacements.push([...ids]),
    (e) => errors.push(e),
    () => ({ load: async () => models.shift()!.root, dispose() {} }),
  );
  await layer.set(f.contract, {} as FileSystemDirectoryHandle, f.source);
  assert.deepEqual([...layer.replacedTargets], [0, 1]);
  assert.deepEqual(errors, []);
  const a = layer.players.get("target0")!.player,
    b = layer.players.get("target1")!.player;
  assert.equal(a.playing, false);
  layer.seek("target0", 2);
  assert.equal(a.tick, 2);
  assert.equal(b.tick, 0);
  layer.setPlaying(true);
  layer.advance(4 / 25);
  assert.equal(a.tick, 0);
  assert.equal(b.tick, 4);
  layer.selectAction("target0", 210);
  layer.setPlaying(true);
  layer.advance(1);
  assert.equal(a.tick, 5);
  assert.equal(a.playing, false);
  const position = a.object.position.toArray();
  layer.seek("target0", 0);
  assert.deepEqual(a.object.position.toArray(), position);
  layer.clear();
  assert.deepEqual(replacements.at(-1), []);
  assert.equal(layer.root.children.length, 0);
  layer.dispose();
});

test("a failed refined model preserves its original target while successful siblings replace theirs", async () => {
  const f = await fixture(),
    errors: string[] = [];
  let count = 0;
  const layer = new MissionStateLayer(
    () => {},
    (e) => errors.push(e),
    () => ({
      load: async () => {
        if (count++ === 0) throw Error("bad hash");
        return asset().root;
      },
      dispose() {},
    }),
  );
  await layer.set(f.contract, {} as FileSystemDirectoryHandle, f.source);
  assert.deepEqual([...layer.replacedTargets], [1]);
  assert.match(errors[0]!, /bad hash/);
  layer.dispose();
});

test("mission switch invalidates in-flight loads and disposes their resources after settlement", async () => {
  const f = await fixture();
  f.contract.targets.splice(1);
  const old = asset();
  let release!: (v: THREE.Group) => void,
    disposed = 0,
    loaderDisposed = 0;
  let started!: () => void;
  const ready = new Promise<void>((resolve) => {
    started = resolve;
  });
  old.mesh.geometry.addEventListener("dispose", () => disposed++);
  const layer = new MissionStateLayer(
    () => {},
    () => {},
    () => ({
      load: () =>
        new Promise((done) => {
          release = done;
          started();
        }),
      dispose() {
        loaderDisposed++;
      },
    }),
  );
  const loading = layer.set(f.contract, {} as FileSystemDirectoryHandle, f.source);
  await ready;
  layer.clear();
  assert.equal(disposed, 0);
  release(old.root);
  await loading;
  assert.equal(disposed, 1);
  assert.equal(loaderDisposed, 1);
  assert.equal(layer.root.children.length, 0);
  assert.equal(layer.replacedTargets.size, 0);
  layer.dispose();
  assert.equal(disposed, 1);
});
