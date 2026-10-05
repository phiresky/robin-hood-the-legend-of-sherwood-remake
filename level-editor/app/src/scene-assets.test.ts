import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";
import * as THREE from "three";
import {
  SceneAssetLoader,
  captureLoadedStateAppearance,
  retainSceneAnimations,
} from "./scene-assets.ts";
import { StateAppearancePlayer } from "./state-appearance-player.ts";
import { parseLevel3D, type SceneAssetSource } from "@rle/shared";

const hash = (bytes: Uint8Array | string) => createHash("sha256").update(bytes).digest("hex");
function fixture() {
  const files = new Map<string, File>();
  const reads = new Map<string, number>();
  const root = (prefix = ""): FileSystemDirectoryHandle =>
    ({
      async getDirectoryHandle(name: string) {
        return root(prefix + name + "/");
      },
      async getFileHandle(name: string) {
        const key = prefix + name;
        reads.set(key, (reads.get(key) ?? 0) + 1);
        if (!files.has(key)) throw new Error("Missing " + key);
        return { getFile: async () => files.get(key)! };
      },
    }) as unknown as FileSystemDirectoryHandle;
  const model = JSON.stringify({
    asset: { version: "2.0" },
    scenes: [{ nodes: [] }],
    scene: 0,
    nodes: [],
  });
  const reference: SceneAssetSource = {
    id: "house",
    role: "objects",
    model: "3d-assets/house.gltf",
    model_sha256: hash(model),
    resources: [{ path: "3d-assets/texture.png", sha256: hash("texture") }],
  };
  files.set(reference.model, new File([model], "house.gltf"));
  files.set(reference.resources[0].path, new File(["texture"], "texture.png"));
  return { files, reads, reference, loader: new SceneAssetLoader(root()) };
}

test("pinned resources are read once and shared across asset loads", async (t) => {
  const f = fixture();
  t.mock.method(GLTFLoader.prototype, "parseAsync", async () => ({ scene: new THREE.Group() }));
  await f.loader.load(f.reference);
  await f.loader.load({ ...f.reference, id: "second-house" });
  assert.equal(f.reads.get("3d-assets/texture.png"), 1);
  f.loader.dispose();
});

test("changed models and resource bytes fail before decoding", async (t) => {
  const f = fixture();
  t.mock.method(GLTFLoader.prototype, "parseAsync", async () => {
    throw new Error("Unexpected decode");
  });
  await assert.rejects(
    f.loader.load({ ...f.reference, model_sha256: "f".repeat(64) }),
    /Scene asset changed/,
  );
  f.files.set("3d-assets/texture.png", new File(["edited"], "texture.png"));
  await assert.rejects(f.loader.load(f.reference), /Scene asset changed/);
  f.loader.dispose();
});

test("map manifests reject old whole-map storage and unsafe asset references", () => {
  const document = {
    version: 1,
    map: "Empty",
    size: null,
    camera: { kind: "oblique-orthographic", elevation_deg: 35 },
    sceneAssets: [],
    objects: [],
    groups: [],
  };
  assert.throws(() => parseLevel3D({ ...document, glb: "map.glb" }), /must be imported/);
  const reference = fixture().reference;
  for (const asset of [
    { ...reference, model: "../outside.gltf" },
    { ...reference, role: "unknown" },
    { ...reference, resources: [...reference.resources, ...reference.resources] },
  ])
    assert.throws(() => parseLevel3D({ ...document, sceneAssets: [asset] }));
});

test("indexed ground lossy models load without receipts or shared source resources", async (t) => {
  const files = new Map<string, File>();
  const root = (prefix = ""): FileSystemDirectoryHandle =>
    ({
      async getDirectoryHandle(name: string) {
        return root(prefix + name + "/");
      },
      async getFileHandle(name: string) {
        if (!files.has(prefix + name)) throw new DOMException(prefix + name, "NotFoundError");
        return { getFile: async () => files.get(prefix + name)! };
      },
    }) as unknown as FileSystemDirectoryHandle;
  const published = new Uint8Array([1, 2, 3]),
    lossy = new Uint8Array([7]);
  const reference: SceneAssetSource = {
    id: "terrain",
    role: "ground",
    model: "3d-assets/terrain/model.glb",
    model_sha256: hash(published),
    resources: [],
  };
  files.set("3d-assets/terrain/lossy.glb", new File([lossy], "lossy.glb"));
  const loaded: number[][] = [];
  t.mock.method(GLTFLoader.prototype, "parseAsync", async (bytes: ArrayBuffer) => {
    loaded.push([...new Uint8Array(bytes)]);
    return { scene: new THREE.Group() };
  });
  const loader = new SceneAssetLoader(
    root(),
    new Map([[reference.model, "3d-assets/terrain/lossy.glb"]]),
  );
  await loader.load(reference);
  assert.deepEqual(loaded, [[7]]);
  files.set(reference.model, new File([published], "model.glb"));
  // Resource-backed models: the self-contained lossy model replaces model and shared payloads.
  const shared = {
    ...reference,
    id: "shared",
    model: "3d-assets/shared/model.glb",
    resources: [{ path: "3d-assets/blobs/atlas.jpg", sha256: hash("atlas") }],
  };
  files.set("3d-assets/shared/lossy.glb", new File([lossy], "lossy.glb"));
  await new SceneAssetLoader(root(), new Map([[shared.model, "3d-assets/shared/lossy.glb"]])).load(
    shared,
  );
  assert.deepEqual(loaded.at(-1), [7]);
  // Without a lossy model entry the pinned model is used unchanged.
  await new SceneAssetLoader(root()).load(reference);
  assert.deepEqual(loaded.at(-1), [1, 2, 3]);
  loader.dispose();
});

test("retained clips survive duplicate display names, extraction and independent clones", async (t) => {
  const f = fixture(),
    scene = new THREE.Group(),
    group = new THREE.Group();
  const first = new THREE.Object3D(),
    second = new THREE.Object3D();
  first.name = "phase";
  second.name = "phase_1";
  scene.add(group);
  group.add(first, second);
  const clip = new THREE.AnimationClip("native", 0.08, [
    new THREE.VectorKeyframeTrack(
      "phase.scale",
      [0, 0.08],
      [1, 1, 1, 0, 0, 0],
      THREE.InterpolateDiscrete,
    ),
    new THREE.VectorKeyframeTrack(
      "phase_1.scale",
      [0, 0.08],
      [0, 0, 0, 1, 1, 1],
      THREE.InterpolateDiscrete,
    ),
  ]);
  t.mock.method(GLTFLoader.prototype, "parseAsync", async () => ({
    scene,
    animations: [clip],
    parser: {
      associations: new Map([
        [first, { nodes: 0 }],
        [second, { nodes: 1 }],
      ]),
      json: { nodes: [{ name: "same/name" }, { name: "same/name" }] },
    },
  }));
  const loaded = await f.loader.load(f.reference);
  assert.deepEqual(
    group.children.map((n) => n.name),
    ["same/name", "same/name"],
  );
  assert.equal(clip.tracks[0]!.name, "phase.scale");
  group.removeFromParent();
  const template = captureLoadedStateAppearance(loaded, group)!;
  const a = new StateAppearancePlayer(template),
    b = new StateAppearancePlayer(template);
  for (const player of [a, b]) player.select("native", { mode: "clamp", terminalTick: 2 });
  a.seek(2);
  assert.equal(a.content.children[1]!.scale.x, 1);
  assert.equal(b.content.children[1]!.scale.x, 0);
  assert.equal(second.scale.x, 1);
  a.dispose();
  b.dispose();
  f.loader.dispose();
});

test("static scenes remain static and shared clips exclude other scene targets", () => {
  const scene = new THREE.Group(),
    other = new THREE.Group();
  scene.name = "selected";
  other.name = "other";
  retainSceneAnimations(scene);
  assert.equal(captureLoadedStateAppearance(scene), undefined);
  const clip = new THREE.AnimationClip("both", 0.08, [
    new THREE.VectorKeyframeTrack("selected.position", [0, 0.08], [0, 0, 0, 1, 0, 0]),
    new THREE.VectorKeyframeTrack("other.position", [0, 0.08], [0, 0, 0, 9, 0, 0]),
  ]);
  retainSceneAnimations(scene, [clip], [scene, other]);
  assert.equal(scene.animations[0]!.tracks.length, 1);
  assert.equal(scene.animations[0]!.tracks[0]!.name, scene.uuid + ".position");
  assert.equal(clip.tracks.length, 2);
});

test("actual binary GLB decoding retains source node identity before names are restored", async () => {
  const times = new Float32Array([0, 2 / 25, 5 / 25]);
  const values = new Float32Array([1, 1, 1, 2, 2, 2, 3, 3, 3]);
  const binary = Buffer.concat([Buffer.from(times.buffer), Buffer.from(values.buffer)]);
  const document = {
    asset: { version: "2.0" },
    scene: 0,
    scenes: [{ nodes: [0] }],
    nodes: [
      { name: "map", children: [1] },
      { name: "group", children: [2, 3] },
      { name: "same/source.name" },
      { name: "same/source.name" },
    ],
    buffers: [{ byteLength: binary.length }],
    bufferViews: [
      { buffer: 0, byteOffset: 0, byteLength: times.byteLength },
      { buffer: 0, byteOffset: times.byteLength, byteLength: values.byteLength },
    ],
    accessors: [
      { bufferView: 0, componentType: 5126, count: 3, type: "SCALAR", min: [0], max: [0.2] },
      { bufferView: 1, componentType: 5126, count: 3, type: "VEC3" },
    ],
    animations: [
      {
        name: "native",
        samplers: [{ input: 0, output: 1, interpolation: "STEP" }],
        channels: [{ sampler: 0, target: { node: 3, path: "scale" } }],
      },
    ],
  };
  const encoded = Buffer.from(JSON.stringify(document));
  const json = Buffer.concat([encoded, Buffer.alloc((4 - (encoded.length % 4)) % 4, 0x20)]);
  const header = Buffer.alloc(20),
    chunk = Buffer.alloc(8);
  header.writeUInt32LE(0x46546c67);
  header.writeUInt32LE(2, 4);
  header.writeUInt32LE(28 + json.length + binary.length, 8);
  header.writeUInt32LE(json.length, 12);
  header.writeUInt32LE(0x4e4f534a, 16);
  chunk.writeUInt32LE(binary.length);
  chunk.writeUInt32LE(0x004e4942, 4);
  const bytes = Buffer.concat([header, json, chunk, binary]);
  const f = fixture();
  const reference = {
    ...f.reference,
    model: "3d-assets/animated.glb",
    model_sha256: hash(bytes),
    resources: [],
  };
  f.files.set(reference.model, new File([bytes], "animated.glb"));
  const scene = await f.loader.load(reference);
  const group = scene.children[0]!.children[0]!;
  assert.deepEqual(
    group.children.map((n) => n.name),
    ["same/source.name", "same/source.name"],
  );
  group.removeFromParent();
  const player = new StateAppearancePlayer(captureLoadedStateAppearance(scene, group)!);
  player.select("native", { mode: "clamp", terminalTick: 5 });
  player.seek(5);
  assert.deepEqual(
    player.content.children.map((n) => n.scale.x),
    [1, 3],
  );
  assert.deepEqual(
    group.children.map((n) => n.scale.x),
    [1, 1],
  );
  player.dispose();
  f.loader.dispose();
});
