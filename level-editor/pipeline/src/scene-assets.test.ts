import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { createHash } from "node:crypto";
import { loadSceneModel } from "./scene-assets.ts";

test("pipeline models receive pinned descriptor appearance rules without changing mesh files", async () => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "descriptor-appearance-"));
  try {
    const model = JSON.stringify({
      asset: { version: "2.0" },
      scene: 0,
      scenes: [{ nodes: [0] }],
      nodes: [{ name: "scenery-body", extras: { scenery: true } }],
    });
    const descriptor = JSON.stringify({
      version: 1,
      kind: "projection-mapped-asset",
      id: "prop",
      name: "Prop",
      source_map: "Authored",
      model: "model.gltf",
      parts: [
        { node: "scenery-body", name: "Body", scenery: true, appearance: { show: ["activate"] } },
      ],
    });
    const hash = (value: string) => createHash("sha256").update(value).digest("hex");
    await fs.writeFile(path.join(root, "model.gltf"), model);
    await fs.writeFile(path.join(root, "asset.json"), descriptor);
    const loaded = await loadSceneModel(root, {
      id: "prop",
      role: "objects",
      model: "model.gltf",
      model_sha256: hash(model),
      descriptor: "asset.json",
      descriptor_sha256: hash(descriptor),
      resources: [],
    });
    assert.deepEqual(loaded.getRoot().listNodes()[0]!.getExtras(), {
      scenery: true,
      reveal_show_when_applied: ["activate"],
    });
    assert.equal(await fs.readFile(path.join(root, "model.gltf"), "utf8"), model);
  } finally {
    await fs.rm(root, { recursive: true, force: true });
  }
});

test("hybrid GLB loads its embedded and pinned external buffers together", async () => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "hybrid-scene-"));
  try {
    await fs.mkdir(path.join(root, "3d-assets/house"), { recursive: true });
    await fs.mkdir(path.join(root, "3d-assets/blobs"));
    const positions = Buffer.from(new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0]).buffer);
    const normals = Buffer.from(new Float32Array([0, 0, 1, 0, 0, 1, 0, 0, 1]).buffer);
    const hash = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");
    const resource = `3d-assets/blobs/${hash(positions)}.bin`;
    const model = {
      asset: { version: "2.0" },
      scene: 0,
      scenes: [{ name: "default", nodes: [0] }],
      nodes: [{ mesh: 0 }],
      meshes: [{ primitives: [{ attributes: { POSITION: 0, NORMAL: 1 } }] }],
      accessors: [
        {
          bufferView: 0,
          componentType: 5126,
          count: 3,
          type: "VEC3",
          min: [0, 0, 0],
          max: [1, 1, 0],
        },
        { bufferView: 1, componentType: 5126, count: 3, type: "VEC3" },
      ],
      bufferViews: [
        { buffer: 1, byteLength: 36 },
        { buffer: 0, byteLength: 36 },
      ],
      buffers: [{ byteLength: 36 }, { byteLength: 36, uri: `../blobs/${hash(positions)}.bin` }],
    };
    const json = Buffer.from(JSON.stringify(model));
    const chunk = Buffer.concat([json, Buffer.alloc((4 - (json.length % 4)) % 4, 32)]);
    const bytes = Buffer.alloc(28 + chunk.length + normals.length);
    [0x46546c67, 2, bytes.length, chunk.length, 0x4e4f534a].forEach((n, i) =>
      bytes.writeUInt32LE(n, i * 4),
    );
    chunk.copy(bytes, 20);
    bytes.writeUInt32LE(36, 20 + chunk.length);
    bytes.writeUInt32LE(0x004e4942, 24 + chunk.length);
    normals.copy(bytes, 28 + chunk.length);
    await fs.writeFile(path.join(root, resource), positions);
    await fs.writeFile(path.join(root, "3d-assets/house/model.glb"), bytes);
    const ref = {
      id: "house",
      role: "objects" as const,
      model: "3d-assets/house/model.glb",
      model_scene: "default",
      model_sha256: hash(bytes),
      resources: [{ path: resource, sha256: hash(positions) }],
    };
    const loaded = await loadSceneModel(root, ref);
    assert.equal(loaded.getRoot().listScenes().length, 1);
    const primitive = loaded.getRoot().listMeshes()[0]!.listPrimitives()[0]!;
    assert.deepEqual(
      [...primitive.getAttribute("POSITION")!.getArray()!],
      [0, 0, 0, 1, 0, 0, 0, 1, 0],
    );
    assert.deepEqual(
      [...primitive.getAttribute("NORMAL")!.getArray()!],
      [0, 0, 1, 0, 0, 1, 0, 0, 1],
    );
    await fs.writeFile(path.join(root, resource), normals);
    await assert.rejects(loadSceneModel(root, ref), /Asset changed/);
  } finally {
    await fs.rm(root, { recursive: true, force: true });
  }
});
