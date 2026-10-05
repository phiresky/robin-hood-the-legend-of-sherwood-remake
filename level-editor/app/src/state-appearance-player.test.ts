import test from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import { captureStateAppearance, StateAppearancePlayer } from "./state-appearance-player.ts";

function fixture() {
  const loaded = new THREE.Group(),
    asset = new THREE.Group();
  asset.name = "loader_asset";
  loaded.add(asset);
  const geometry = new THREE.BoxGeometry(),
    material = new THREE.MeshBasicMaterial();
  const phases = [0, 1, 2].map((i) => {
    const node = new THREE.Mesh(geometry, material);
    node.name = `loader_phase_${i}`;
    node.scale.setScalar(i === 0 ? 1 : 0);
    asset.add(node);
    return node;
  });
  // Delays 1, 2, 0 mean phases begin at ticks 0, 2, 5 and loop after six ticks.
  const times = [0, 2 / 25, 5 / 25, 6 / 25];
  const tracks = phases.map(
    (node, i) =>
      new THREE.VectorKeyframeTrack(
        node.name + ".scale",
        times,
        [0, 1, 2, 0].flatMap((phase) => [
          phase === i ? 1 : 0,
          phase === i ? 1 : 0,
          phase === i ? 1 : 0,
        ]),
        THREE.InterpolateDiscrete,
      ),
  );
  tracks.push(
    new THREE.VectorKeyframeTrack(
      "loader_asset.position",
      times,
      [0, 0, 0, 2, 0, 0, 5, 0, 0, 0, 0, 0],
      THREE.InterpolateDiscrete,
    ),
  );
  const clip = new THREE.AnimationClip("native", -1, tracks);
  const template = captureStateAppearance(loaded, [clip], asset);
  return { loaded, asset, phases, clip, template, geometry, material };
}
function phase(player: StateAppearancePlayer) {
  const active = player.content.children.flatMap((node, i) => (node.scale.x > 0.5 ? [i] : []));
  assert.equal(active.length, 1);
  return active[0];
}

test("all native boundaries and loop wraps select exact STEP phases despite float32 seconds", () => {
  const f = fixture(),
    player = new StateAppearancePlayer(f.template);
  assert.ok(f.clip.tracks[0]!.times[2]! > 5 / 25, "fixture must expose float32 boundary rounding");
  player.select("native", { mode: "loop", cycleTicks: 6 });
  for (let tick = 0; tick < 30; tick++) {
    player.seek(tick);
    assert.equal(player.tick, tick % 6);
    assert.equal(phase(player), [0, 0, 1, 1, 1, 2][tick % 6]);
  }
  player.dispose();
});

test("frozen targets stop upon entering the last frame and can seek backwards after clamping", () => {
  const player = new StateAppearancePlayer(fixture().template);
  player.select("native", { mode: "clamp", terminalTick: 5 });
  player.play();
  player.advance(4 / 25);
  assert.equal(phase(player), 1);
  assert.equal(player.playing, true);
  player.advance(1 / 25);
  assert.equal(phase(player), 2);
  assert.equal(player.tick, 5);
  assert.equal(player.playing, false);
  player.seek(100);
  assert.equal(phase(player), 2);
  player.play();
  assert.equal(player.playing, false);
  player.seek(0);
  assert.equal(phase(player), 0);
  player.play();
  player.advance(10);
  assert.equal(phase(player), 2);
  player.dispose();
});

test("restored duplicate display names and extracted groups retain independent clone bindings", () => {
  const f = fixture();
  f.asset.name = "Display / cart";
  for (const node of f.phases) node.name = "same restored name";
  f.asset.removeFromParent();
  const a = new StateAppearancePlayer(f.template),
    b = new StateAppearancePlayer(f.template);
  a.object.position.set(100, 200, 300);
  a.select("native", { mode: "loop", cycleTicks: 6 });
  b.select("native", { mode: "loop", cycleTicks: 6 });
  a.seek(2);
  b.seek(5);
  assert.equal(phase(a), 1);
  assert.equal(phase(b), 2);
  assert.equal(a.content.position.x, 2);
  assert.equal(b.content.position.x, 5);
  assert.deepEqual(a.object.position.toArray(), [100, 200, 300]);
  assert.equal(f.asset.position.x, 0);
  assert.deepEqual(
    f.phases.map((n) => n.scale.x),
    [1, 0, 0],
  );
  assert.equal((a.content.children[0] as THREE.Mesh).geometry, f.geometry);
  a.dispose();
  assert.equal(phase(b), 2);
  b.seek(0);
  assert.equal(phase(b), 0);
  b.dispose();
});

test("fractional elapsed updates, pause and a direct seek agree at native tick boundaries", () => {
  const a = new StateAppearancePlayer(fixture().template);
  const b = new StateAppearancePlayer(fixture().template);
  for (const p of [a, b]) {
    p.select("native", { mode: "loop", cycleTicks: 6 });
    p.play();
  }
  for (let i = 0; i < 40; i++) a.advance(0.01);
  b.advance(0.4);
  assert.equal(a.tick, 4);
  assert.equal(a.tick, b.tick);
  assert.equal(phase(a), phase(b));
  a.pause();
  a.advance(100);
  assert.equal(a.tick, 4);
  a.play();
  a.advance(0.08);
  assert.equal(a.tick, 0);
  assert.equal(phase(a), 0);
  a.dispose();
  b.dispose();
});

test("capture rejects missing, ambiguous and out-of-scope targets instead of silently dropping tracks", () => {
  const f = fixture();
  const make = (target: string) =>
    new THREE.AnimationClip("bad", -1, [
      new THREE.VectorKeyframeTrack(target + ".scale", [0], [1, 1, 1]),
    ]);
  assert.throws(
    () => captureStateAppearance(f.loaded, [make("missing")], f.asset),
    /Missing or ambiguous/,
  );
  const duplicate = new THREE.Group();
  duplicate.name = f.phases[0]!.name;
  f.loaded.add(duplicate);
  assert.throws(
    () => captureStateAppearance(f.loaded, [make(duplicate.name)], f.asset),
    /Missing or ambiguous/,
  );
  assert.throws(
    () => captureStateAppearance(f.loaded, [make(duplicate.uuid)], f.asset),
    /outside selected asset/,
  );
  f.asset.children.reverse();
  assert.throws(() => new StateAppearancePlayer(f.template), /hierarchy changed/);
});

test("invalid clocks and shared-material tracks fail before changing a running player", () => {
  const f = fixture();
  const materialClip = new THREE.AnimationClip("material", -1, [
    new THREE.NumberKeyframeTrack(f.phases[0]!.name + ".material.opacity", [0], [0]),
  ]);
  assert.throws(() => captureStateAppearance(f.loaded, [materialClip], f.asset), /Unsupported/);
  const offGrid = new THREE.AnimationClip("off-grid", -1, [
    new THREE.VectorKeyframeTrack(f.phases[0]!.name + ".scale", [0, 0.05], [1, 1, 1, 0, 0, 0]),
  ]);
  assert.throws(() => captureStateAppearance(f.loaded, [offGrid], f.asset), /25 Hz grid/);
  const player = new StateAppearancePlayer(f.template);
  assert.throws(() => player.play(), /Select/);
  player.select("native", { mode: "loop", cycleTicks: 6 });
  player.play();
  player.seek(2);
  assert.throws(() => player.select("native", { mode: "loop", cycleTicks: 0 }), /positive/);
  assert.throws(() => player.select("missing", { mode: "loop", cycleTicks: 6 }), /Unknown/);
  assert.throws(() => player.advance(NaN), /finite/);
  assert.throws(() => player.seek(2.5), /integer/);
  assert.equal(phase(player), 1);
  assert.equal(player.playing, true);
  let disposed = false;
  f.geometry.addEventListener("dispose", () => {
    disposed = true;
  });
  player.dispose();
  player.dispose();
  assert.equal(disposed, false);
  assert.throws(() => player.seek(0), /disposed/);
});

test("real GLTFLoader bindings survive restored source names and selected-group extraction", async () => {
  const { GLTFLoader } = await import("three/examples/jsm/loaders/GLTFLoader.js");
  const times = new Float32Array([0, 2 / 25, 5 / 25, 6 / 25]);
  const outputs = [0, 1, 2].map(
    (i) =>
      new Float32Array(
        [0, 1, 2, 0].flatMap((p) => [p === i ? 1 : 0, p === i ? 1 : 0, p === i ? 1 : 0]),
      ),
  );
  const chunks = [times, ...outputs].map((a) => Buffer.from(a.buffer));
  const binary = Buffer.concat(chunks);
  let offset = 0;
  const bufferViews = chunks.map((chunk) => {
    const view = { buffer: 0, byteOffset: offset, byteLength: chunk.length };
    offset += chunk.length;
    return view;
  });
  const gltf = {
    asset: { version: "2.0" },
    buffers: [{ byteLength: binary.length }],
    bufferViews,
    accessors: [
      { bufferView: 0, componentType: 5126, count: 4, type: "SCALAR", min: [0], max: [0.24] },
      ...outputs.map((_, i) => ({
        bufferView: i + 1,
        componentType: 5126,
        count: 4,
        type: "VEC3",
      })),
    ],
    nodes: [
      { name: "map", children: [1] },
      { name: "asset", children: [2, 3, 4] },
      ...outputs.map((_, i) => ({
        name: "same/source.name",
        scale: [i === 0 ? 1 : 0, i === 0 ? 1 : 0, i === 0 ? 1 : 0],
      })),
    ],
    scenes: [{ nodes: [0] }],
    scene: 0,
    animations: [
      {
        name: "native",
        samplers: outputs.map((_, i) => ({ input: 0, output: i + 1, interpolation: "STEP" })),
        channels: outputs.map((_, i) => ({ sampler: i, target: { node: i + 2, path: "scale" } })),
      },
    ],
  };
  const json = Buffer.from(JSON.stringify(gltf));
  const padded = Buffer.concat([json, Buffer.alloc((4 - (json.length % 4)) % 4, 0x20)]);
  const header = Buffer.alloc(20);
  header.writeUInt32LE(0x46546c67);
  header.writeUInt32LE(2, 4);
  header.writeUInt32LE(20 + padded.length + 8 + binary.length, 8);
  header.writeUInt32LE(padded.length, 12);
  header.writeUInt32LE(0x4e4f534a, 16);
  const binaryHeader = Buffer.alloc(8);
  binaryHeader.writeUInt32LE(binary.length);
  binaryHeader.writeUInt32LE(0x004e4942, 4);
  const bytes = Buffer.concat([header, padded, binaryHeader, binary]);
  const result = await new GLTFLoader().parseAsync(
    bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength),
    "",
  );
  const group = result.scene.children[0]!.children[0]!;
  const template = captureStateAppearance(result.scene, result.animations, group);
  result.scene.traverse((node) => {
    const index = result.parser.associations.get(node)?.nodes;
    if (index !== undefined) node.name = gltf.nodes[index]!.name;
  });
  group.removeFromParent();
  const player = new StateAppearancePlayer(template);
  player.select("native", { mode: "loop", cycleTicks: 6 });
  player.seek(5);
  assert.equal(phase(player), 2);
  assert.deepEqual(
    player.content.children.map((n) => n.name),
    Array(3).fill("same/source.name"),
  );
  player.dispose();
});
