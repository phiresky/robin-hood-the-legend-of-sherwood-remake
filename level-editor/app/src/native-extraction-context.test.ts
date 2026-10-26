import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";
import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import * as THREE from "three";
import {
  bindNativeExtractionContext,
  prepareNativeExtractionContext,
} from "./native-extraction-context.ts";
import type { MissionStateContract } from "../../shared/src/mission-state.ts";
const hash = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");
async function fixture(multiplePrimitives = false) {
  const plane = new THREE.PlaneGeometry(2, 2);
  const chunks: Buffer[] = [],
    views: object[] = [],
    accessors: object[] = [];
  let byteLength = 0;
  const add = (array: Float32Array | Uint16Array, size: number) => {
    const padding = -byteLength & 3;
    if (padding) {
      chunks.push(Buffer.alloc(padding));
      byteLength += padding;
    }
    const raw = Buffer.from(array.buffer, array.byteOffset, array.byteLength);
    views.push({ buffer: 0, byteOffset: byteLength, byteLength: raw.length });
    chunks.push(raw);
    byteLength += raw.length;
    accessors.push({
      bufferView: views.length - 1,
      componentType: array instanceof Uint16Array ? 5123 : 5126,
      count: array.length / size,
      type: size === 1 ? "SCALAR" : `VEC${size}`,
      ...(size === 3 ? { min: [-1, -1, 0], max: [1, 1, 0] } : {}),
    });
    return accessors.length - 1;
  };
  const position = add(plane.attributes.position!.array as Float32Array, 3),
    uv = add(plane.attributes.uv!.array as Float32Array, 2),
    indices = add(plane.index!.array as Uint16Array, 1),
    morph = add(new Float32Array(12), 3);
  const primitive = {
    attributes: { POSITION: position, TEXCOORD_0: uv },
    indices,
    targets: [{ POSITION: morph }, { POSITION: morph }],
  };
  const parserJson = {
    asset: { version: "2.0" },
    scene: 0,
    scenes: [{ nodes: [0, 1] }],
    nodes: [
      { mesh: 0, translation: [0, 0, 1] },
      { mesh: 1, translation: [0, 0, 2] },
    ],
    meshes: [
      { primitives: multiplePrimitives ? [primitive, structuredClone(primitive)] : [primitive] },
      { primitives: [primitive] },
    ],
    buffers: [{ byteLength }],
    bufferViews: views,
    accessors,
  };
  const json = Buffer.from(JSON.stringify(parserJson)),
    length = Math.ceil(json.length / 4) * 4;
  const binary = Buffer.concat(chunks),
    binLength = Math.ceil(binary.length / 4) * 4;
  const modelBytes = Buffer.alloc(28 + length + binLength, 32);
  modelBytes.writeUInt32LE(0x46546c67, 0);
  modelBytes.writeUInt32LE(2, 4);
  modelBytes.writeUInt32LE(modelBytes.length, 8);
  modelBytes.writeUInt32LE(length, 12);
  modelBytes.writeUInt32LE(0x4e4f534a, 16);
  json.copy(modelBytes, 20);
  modelBytes.writeUInt32LE(binLength, 20 + length);
  modelBytes.writeUInt32LE(0x004e4942, 24 + length);
  binary.copy(modelBytes, 28 + length);
  const loaded = await new GLTFLoader().parseAsync(
    modelBytes.buffer.slice(modelBytes.byteOffset, modelBytes.byteOffset + modelBytes.byteLength),
    "",
  );
  const object = loaded.scene;
  object.position.set(4, 6, 8);
  const owner = object.children[0] as THREE.Mesh<THREE.BufferGeometry, THREE.MeshBasicMaterial>;
  const receiver = object.children[1] as THREE.Mesh<THREE.BufferGeometry, THREE.MeshBasicMaterial>;
  owner.material = new THREE.MeshBasicMaterial();
  receiver.material = new THREE.MeshBasicMaterial({
    map: new THREE.DataTexture(new Uint8Array([255, 255, 255, 0]), 1, 1),
    alphaTest: 0.5,
    side: THREE.FrontSide,
  });
  receiver.morphTargetInfluences = [0.25, 0.75];
  const binding = {
    source_kind: "map-animation",
    target_index: 5,
    target_sha256: "b".repeat(64),
    source: { model_sha256: hash(modelBytes) },
  } as MissionStateContract["targets"][number];
  const manifest = {
    version: 1,
    model_sha256: hash(modelBytes),
    owner: { kind: "map-animation", index: 5, sha256: binding.target_sha256 },
    components: [
      { node: 0, role: "source-owner" },
      { node: 1, role: "static-receiver" },
    ],
  };
  const manifestBytes = Buffer.from(JSON.stringify(manifest));
  const associations = loaded.parser.associations as Map<
    THREE.Object3D,
    { nodes: number; meshes: number; primitives: number }
  >;
  return {
    object,
    owner,
    receiver,
    manifest,
    input: {
      object,
      binding,
      modelBytes,
      manifestBytes,
      manifestSha256: hash(manifestBytes),
      parserJson: loaded.parser.json,
      associations,
    },
  };
}

test("pinned receiver context preserves world pose, morphs, alpha maps, sidedness and original materials", async () => {
  const f = await fixture(),
    context = await bindNativeExtractionContext(f.input);
  const prepared = prepareNativeExtractionContext(context, f.object, f.input.binding);
  const receiver = prepared.depth.children[1] as THREE.Mesh<
    THREE.BufferGeometry,
    THREE.MeshBasicMaterial
  >;
  assert.deepEqual(prepared.depth.matrix.elements, f.object.matrixWorld.elements);
  assert.deepEqual(receiver.position.toArray(), f.receiver.position.toArray());
  assert.deepEqual(receiver.morphTargetInfluences, [0.25, 0.75]);
  assert.equal(receiver.geometry, f.receiver.geometry);
  assert.equal(receiver.material.map, f.receiver.material.map);
  assert.equal(receiver.material.alphaTest, 0.5);
  assert.equal(receiver.material.side, THREE.FrontSide);
  assert.equal(receiver.material.colorWrite, false);
  assert.equal(receiver.material.depthWrite, true);
  assert.equal(prepared.depth.children[0]!.layers.mask, 0);
  assert.equal(prepared.owner.children[1]!.layers.mask, 0);
  assert.equal(f.receiver.layers.mask, 1);
  assert.equal(f.receiver.material.colorWrite, true);
  let disposed = 0;
  receiver.material.addEventListener("dispose", () => disposed++);
  prepared.dispose();
  assert.equal(disposed, 1);
});
test("pin drift, unrelated family and missing or duplicate component authority are rejected", async () => {
  const stale = await fixture();
  stale.input.manifestSha256 = "f".repeat(64);
  await assert.rejects(bindNativeExtractionContext(stale.input), /pin mismatch/);
  const changed = await fixture();
  changed.input.modelBytes[21] ^= 1;
  await assert.rejects(bindNativeExtractionContext(changed.input), /pin mismatch/);
  for (const change of [
    (m: Awaited<ReturnType<typeof fixture>>["manifest"]) => {
      m.owner.index = 6;
    },
    (m: Awaited<ReturnType<typeof fixture>>["manifest"]) => {
      m.components[1]!.role = "unrelated";
    },
    (m: Awaited<ReturnType<typeof fixture>>["manifest"]) => {
      m.components.pop();
    },
    (m: Awaited<ReturnType<typeof fixture>>["manifest"]) => {
      m.components.push(m.components[0]!);
    },
  ]) {
    const f = await fixture();
    change(f.manifest);
    f.input.manifestBytes = Buffer.from(JSON.stringify(f.manifest));
    f.input.manifestSha256 = hash(f.input.manifestBytes);
    await assert.rejects(bindNativeExtractionContext(f.input));
  }
});
test("foreign parser, source mesh substitution and unverified contexts fail closed", async () => {
  const f = await fixture();
  f.input.parserJson.nodes[1]!.mesh = 0;
  await assert.rejects(bindNativeExtractionContext(f.input), /parser differs/);
  const g = await fixture();
  g.input.associations.set(g.receiver, { nodes: 1, meshes: 0, primitives: 0 });
  await assert.rejects(bindNativeExtractionContext(g.input), /association differs/);
  assert.throws(
    () =>
      prepareNativeExtractionContext(
        {
          prepare: () => {
            throw Error("called");
          },
        },
        g.object,
        g.input.binding,
      ),
    /Unverified/,
  );
});
test("late binding or loaded-family changes cannot reuse a verified context", async () => {
  const f = await fixture(),
    context = await bindNativeExtractionContext(f.input);
  assert.throws(
    () =>
      prepareNativeExtractionContext(context, f.object, { ...f.input.binding, target_index: 6 }),
    /binding changed/,
  );
  f.object.add(new THREE.Mesh());
  assert.throws(
    () => prepareNativeExtractionContext(context, f.object, f.input.binding),
    /loaded family changed/,
  );
});
test("receiver preparation failure leaves original visibility, pose and material intact", async () => {
  const f = await fixture(),
    context = await bindNativeExtractionContext(f.input);
  f.receiver.material.transparent = true;
  assert.throws(
    () => prepareNativeExtractionContext(context, f.object, f.input.binding),
    /unsupported receiver depth/,
  );
  assert.equal(f.receiver.material.transparent, true);
  assert.equal(f.receiver.material.colorWrite, true);
  assert.equal(f.receiver.layers.mask, 1);
  assert.deepEqual(f.receiver.position.toArray(), [0, 0, 2]);
});

// A one-pixel ray oracle exercises the saved partition and alpha-tested depth
// materials. Actual GPU coverage/color fidelity remains a separate capture gate.
for (const [label, receiverZ, receiverAlpha, expectedVisible] of [
  ["opaque foreground receiver", 2, 255, false],
  ["foreground receiver alpha hole", 2, 0, true],
  ["opaque receiver behind owner", 0, 255, true],
] as const) {
  test(`extracted owner visibility: ${label}`, async () => {
    const f = await fixture();
    f.receiver.position.z = receiverZ;
    (f.receiver.material.map!.image.data as Uint8Array)[3] = receiverAlpha;
    const context = await bindNativeExtractionContext(f.input);
    const prepared = prepareNativeExtractionContext(context, f.object, f.input.binding);
    try {
      prepared.owner.updateMatrixWorld(true);
      prepared.depth.updateMatrixWorld(true);
      const ray = new THREE.Raycaster(new THREE.Vector3(4, 6, 100), new THREE.Vector3(0, 0, -1));
      const covered = (root: THREE.Object3D) =>
        ray.intersectObject(root, true).filter((hit) => {
          const mesh = hit.object as THREE.Mesh<THREE.BufferGeometry, THREE.MeshBasicMaterial>;
          const material = mesh.material;
          const alpha = material.map ? (material.map.image.data as Uint8Array)[3]! / 255 : 1;
          return alpha >= material.alphaTest;
        });
      const owner = covered(prepared.owner)[0],
        receiver = covered(prepared.depth)[0];
      assert.ok(owner);
      assert.equal(!receiver || owner.distance <= receiver.distance, expectedVisible);
      assert.equal(f.receiver.material.colorWrite, true);
    } finally {
      prepared.dispose();
    }
  });
}

test("actual GLTFLoader accessors reject geometry mismatches and in-place mutation while allowing pose weights", async () => {
  const f = await fixture();
  f.receiver.geometry.attributes.position!.setX(0, 123);
  await assert.rejects(bindNativeExtractionContext(f.input), /loaded accessor differs/);
  const g = await fixture(),
    context = await bindNativeExtractionContext(g.input);
  g.receiver.morphTargetInfluences = [0.6, 0.4];
  prepareNativeExtractionContext(context, g.object, g.input.binding).dispose();
  g.receiver.geometry.attributes.uv!.setX(0, 0.2);
  assert.throws(
    () => prepareNativeExtractionContext(context, g.object, g.input.binding),
    /immutable accessor changed/,
  );
  const h = await fixture();
  h.input.associations.get(h.receiver)!.primitives = 8;
  await assert.rejects(bindNativeExtractionContext(h.input), /unsupported primitive association/);
});

test("every primitive of a real multi-primitive GLB node must load exactly once", async () => {
  const f = await fixture(true);
  const node = f.object.children[0]!;
  assert.equal(node.children.length, 2);
  node.remove(node.children[1]!);
  await assert.rejects(bindNativeExtractionContext(f.input), /missing loaded primitive/);
  const g = await fixture(true),
    group = g.object.children[0]!;
  const duplicate = group.children[0]!.clone();
  g.input.associations.set(duplicate, { ...g.input.associations.get(group.children[0]!)! });
  group.add(duplicate);
  await assert.rejects(bindNativeExtractionContext(g.input), /duplicate loaded primitive/);
});
test("morph weights may animate but relative morph semantics are immutable", async () => {
  const f = await fixture(),
    context = await bindNativeExtractionContext(f.input);
  f.receiver.geometry.morphTargetsRelative = false;
  assert.throws(
    () => prepareNativeExtractionContext(context, f.object, f.input.binding),
    /morph semantics changed/,
  );
});

test("loader rebinds a complete placed copy and rejects substituted geometry", async () => {
  const { NativeExtractionLoader } = await import("./native-extraction-loader.ts");
  const f = await fixture(),
    loader = new NativeExtractionLoader(async () => f.input.manifestBytes);
  loader.retain(f.input);
  const placed = f.object.clone(true);
  placed.position.set(80, 20, 30);
  const pin = { path: "roles/tree.json", sha256: f.input.manifestSha256 };
  const wrapper = new THREE.Group(); wrapper.position.set(5, 6, 7); wrapper.add(placed);
  const context = await loader.bind(f.object, placed, f.input.binding, pin, () => true, wrapper);
  const prepared = prepareNativeExtractionContext(context, wrapper, f.input.binding);
  assert.equal(prepared.owner.children[0]!.children.length, placed.children.length);
  prepared.dispose();
  const changed = f.object.clone(true);
  (changed.children[0] as THREE.Mesh).geometry = new THREE.BoxGeometry();
  await assert.rejects(loader.bind(f.object, changed, f.input.binding, pin, () => true));
  const missing = f.object.clone(true);
  missing.remove(missing.children[0]!);
  await assert.rejects(
    loader.bind(f.object, missing, f.input.binding, pin, () => true),
    /hierarchy differs/,
  );
});

test("loader retirement and failed manifest reads never install a fallback context", async () => {
  const { NativeExtractionLoader } = await import("./native-extraction-loader.ts");
  const f = await fixture(),
    pin = { path: "roles/tree.json", sha256: f.input.manifestSha256 };
  let release!: (bytes: Uint8Array) => void;
  const loader = new NativeExtractionLoader(
    () =>
      new Promise((resolve) => {
        release = resolve;
      }),
  );
  loader.retain(f.input);
  const pending = loader.bind(f.object, f.object, f.input.binding, pin, () => true);
  loader.dispose();
  release(f.input.manifestBytes);
  await assert.rejects(pending, /retired/);
  const failure = new NativeExtractionLoader(async () => {
    throw new Error("read failed");
  });
  failure.retain(f.input);
  await assert.rejects(
    failure.bind(f.object, f.object, f.input.binding, pin, () => true),
    /read failed/,
  );
  await assert.rejects(
    failure.bind(f.object, f.object, f.input.binding, { ...pin, path: "../bad" }, () => true),
    /Invalid/,
  );
  const corrupt = new NativeExtractionLoader(async () => new Uint8Array([0]));
  corrupt.retain(f.input);
  await assert.rejects(
    corrupt.bind(f.object, f.object, f.input.binding, pin, () => true),
    /pin mismatch/,
  );
  let active = true;
  const late = new NativeExtractionLoader(async () => {
    active = false;
    return f.input.manifestBytes;
  });
  late.retain(f.input);
  await assert.rejects(
    late.bind(f.object, f.object, f.input.binding, pin, () => active),
    /retired/,
  );
});

test("pinned session isolates arbitrary source writes, follows poses, and retires explicitly", async () => {
  const { createNativeExtractionSession } = await import("./native-extraction-session.ts");
  const f = await fixture();
  const context = await bindNativeExtractionContext(f.input);
  const session = await createNativeExtractionSession(context, f.object, f.input.binding);
  const draws: THREE.Scene[] = [];
  const renderer = {
    xr: { enabled: false }, capabilities: { maxTextureSize: 4096 },
    shadowMap: { enabled: false, needsUpdate: false }, autoClear: true,
    getRenderTarget: () => null, getActiveCubeFace: () => 0, getActiveMipmapLevel: () => 0,
    getViewport: (v: THREE.Vector4) => v, getScissor: (v: THREE.Vector4) => v,
    getScissorTest: () => false, getClearColor: (c: THREE.Color) => c, getClearAlpha: () => 0,
    setRenderTarget() {}, setViewport() {}, setScissor() {}, setScissorTest() {}, setClearColor() {}, clear() {},
    render(scene: THREE.Scene) { draws.push(scene); }, readRenderTargetPixels() {},
  } as unknown as THREE.WebGLRenderer;
  const frame = { x: 0, y: 0, width: 1, height: 1 };
  session.render(renderer, 35, frame);
  const privateOwner = draws[1]!.children[0]!.children[0] as THREE.Mesh;
  const privateReceiver = draws[0]!.children[0]!.children[1] as THREE.Mesh<THREE.BufferGeometry, THREE.MeshBasicMaterial>;
  const originalPosition = privateOwner.geometry.attributes.position!.getX(0);
  const originalTexel = privateReceiver.material.map!.image.data[0];
  const oldGeometry = f.owner.geometry;
  const originalMorph = privateOwner.geometry.morphAttributes.position![0]!.getX(0);
  let originalDisposed = 0, textureDisposed = 0, materialDisposed = 0;
  oldGeometry.addEventListener("dispose", () => originalDisposed++);
  privateReceiver.material.map!.addEventListener("dispose", () => textureDisposed++);
  privateReceiver.material.addEventListener("dispose", () => materialDisposed++);
  let disposed = 0; privateOwner.geometry.addEventListener("dispose", () => disposed++);
  f.owner.geometry.attributes.position!.setX(0, 1234);
  f.owner.geometry.morphAttributes.position![0]!.setX(0, 999);
  f.receiver.material.map!.image.data[0] = 12;
  f.receiver.material.opacity = 0.2;
  f.owner.position.x = 10;
  f.owner.morphTargetInfluences = [0.5, 0.5];
  session.render(renderer, 35, frame);
  assert.equal(privateOwner.geometry.attributes.position!.getX(0), originalPosition);
  assert.equal(privateReceiver.material.map!.image.data[0], originalTexel);
  assert.equal(privateReceiver.material.opacity, 1);
  assert.equal(privateOwner.geometry.morphAttributes.position![0]!.getX(0), originalMorph);
  assert.equal(privateOwner.matrix.elements[12], 10);
  assert.deepEqual(privateOwner.morphTargetInfluences, [0.5, 0.5]);
  assert.throws(() => prepareNativeExtractionContext(context, f.object, f.input.binding), /immutable accessor changed/);
  assert.notEqual(privateOwner.geometry, oldGeometry);
  session.invalidate("supported edit");
  assert.equal(disposed, 1);
  assert.throws(() => session.render(renderer, 35, frame), /retired: supported edit/);
  session.dispose(); assert.equal(disposed, 1);
  assert.equal(originalDisposed, 0);
  assert.equal(textureDisposed, 1);
  assert.equal(materialDisposed, 1);
});

test("pinned session rejects hierarchy, resource identity and binding changes", async () => {
  const { createNativeExtractionSession } = await import("./native-extraction-session.ts");
  const f = await fixture();
  const session = await createNativeExtractionSession(await bindNativeExtractionContext(f.input), f.object, f.input.binding);
  f.object.add(new THREE.Group());
  assert.throws(() => session.synchronize(), /hierarchy changed/); f.object.remove(f.object.children.at(-1)!);
  const geometry = f.owner.geometry; f.owner.geometry = geometry.clone();
  assert.throws(() => session.synchronize(), /resources changed/); f.owner.geometry.dispose(); f.owner.geometry = geometry;
  f.input.binding.target_sha256 = "c".repeat(64);
  assert.throws(() => session.synchronize(), /binding changed/);
  session.dispose();
});

test("pinned session rejects geometry edits racing asynchronous snapshot setup", async () => {
  const { createNativeExtractionSession } = await import("./native-extraction-session.ts");
  const f = await fixture();
  const context = await bindNativeExtractionContext(f.input);
  const pending = createNativeExtractionSession(context, f.object, f.input.binding);
  f.owner.geometry.attributes.position!.setY(0, 456);
  await assert.rejects(pending, /immutable accessor changed/);
});

test("retiring strict verification leaves an independent session usable and rejects old strict handles", async () => {
  const { createNativeExtractionSession } = await import("./native-extraction-session.ts");
  const { retireNativeExtractionContext } = await import("./native-extraction-context.ts");
  const f = await fixture();
  const context = await bindNativeExtractionContext(f.input);
  const session = await createNativeExtractionSession(context, f.object, f.input.binding);
  retireNativeExtractionContext(context);
  assert.throws(() => context.prepare(f.object, f.input.binding), /retired/);
  assert.throws(() => prepareNativeExtractionContext(context, f.object, f.input.binding), /Unverified/);
  f.owner.geometry.attributes.position!.setX(0, 1234);
  f.owner.position.x = 7;
  assert.doesNotThrow(() => session.synchronize());
  retireNativeExtractionContext(context);
  session.dispose();
});

test("binding owns GLB, manifest and parser inputs before asynchronous hashing", async () => {
  const f = await fixture();
  const pending = bindNativeExtractionContext(f.input);
  f.input.modelBytes.fill(0);
  f.input.manifestBytes.fill(0);
  f.input.parserJson.nodes.length = 0;
  const context = await pending;
  prepareNativeExtractionContext(context, f.object, f.input.binding).dispose();
  f.owner.geometry.attributes.position!.setX(0, 321);
  assert.throws(() => prepareNativeExtractionContext(context, f.object, f.input.binding), /immutable accessor changed/);
});
