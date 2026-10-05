import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { meshoptReadable } from "./meshopt-glb.ts";
import { Document, NodeIO } from "@gltf-transform/core";
import { generatePreview, previewFingerprint, previewTextureSize } from "./preview-model.ts";
import sharp from "sharp";

test("preview texture edge follows source texels: /8, multiple of 16, clamped 32-512", () => {
  assert.equal(previewTextureSize(112 * 112), 32);
  assert.equal(previewTextureSize(848 * 848), 112);
  assert.equal(previewTextureSize(3184 * 3184), 400);
  assert.equal(previewTextureSize(2944 * 2176), 320);
  assert.equal(previewTextureSize(8192 * 8192), 512);
  assert.throws(() => previewTextureSize(0), /no texture/);
});

test("preview compression keeps both axes positive for thin texture strips", async (t) => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "preview-strips-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  for (const [width, height, expected] of [
    [2090, 1, [32, 1]],
    [1, 1265, [1, 32]],
    [64, 64, [32, 32]],
  ] as const) {
    const doc = new Document();
    const buffer = doc.createBuffer();
    const image = await sharp({ create: { width, height, channels: 4, background: "#63884a" } })
      .png().toBuffer();
    const texture = doc.createTexture().setImage(image).setMimeType("image/png");
    const material = doc.createMaterial().setBaseColorTexture(texture);
    const positions = doc.createAccessor().setType("VEC3")
      .setArray(new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0])).setBuffer(buffer);
    const uv = doc.createAccessor().setType("VEC2")
      .setArray(new Float32Array([0, 0, 1, 0, 0, 1])).setBuffer(buffer);
    const primitive = doc.createPrimitive().setAttribute("POSITION", positions)
      .setAttribute("TEXCOORD_0", uv).setMaterial(material);
    doc.createScene().addChild(doc.createNode().setMesh(doc.createMesh().addPrimitive(primitive)));
    const input = path.join(root, `${width}x${height}.glb`);
    await new NodeIO().write(input, doc);
    const original = await fs.readFile(input);
    const { bytes, edge } = await generatePreview(input);
    const output = await new NodeIO()
      .registerExtensions((await import("@gltf-transform/extensions")).ALL_EXTENSIONS)
      .registerDependencies({ "meshopt.decoder": (await import("meshoptimizer")).MeshoptDecoder })
      .readBinary(meshoptReadable(bytes));
    assert.equal(edge, 32);
    const actual = output.getRoot().listTextures();
    assert.equal(actual.length, 1);
    assert.deepEqual(actual[0]!.getSize(), expected);
    assert.equal(actual[0]!.getMimeType(), "image/avif");
    assert.deepEqual(await fs.readFile(input), original);
  }
});

test("internal generator returns deterministic meshopt preview bytes; standalone CLI is rejected", async (t) => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "preview-model-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const doc = new Document();
  const buffer = doc.createBuffer();
  const positions = doc
    .createAccessor()
    .setType("VEC3")
    .setArray(new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0]))
    .setBuffer(buffer);
  const indices = doc
    .createAccessor()
    .setType("SCALAR")
    .setArray(new Uint16Array([0, 1, 2]))
    .setBuffer(buffer);
  const primitive = doc.createPrimitive().setAttribute("POSITION", positions).setIndices(indices);
  doc.createScene().addChild(doc.createNode().setMesh(doc.createMesh().addPrimitive(primitive)));
  const input = path.join(root, "model.glb");
  await new NodeIO().write(input, doc);
  const run = promisify(execFile);
  const script = new URL("./preview-model.ts", import.meta.url).pathname;
  await assert.rejects(run(process.execPath, [script, input, path.join(root, "a.glb")]), /Internal module/);
  const first = await generatePreview(input);
  const second = await generatePreview(input);
  assert.equal(first.edge, null);
  assert.deepEqual(first.bytes, second.bytes);
  const bytes = Buffer.from(first.bytes);
  assert.equal(bytes.toString("ascii", 0, 4), "glTF");
  const json = JSON.parse(bytes.toString("utf8", 20, 20 + bytes.readUInt32LE(12)));
  assert.ok(json.extensionsUsed.includes("KHR_meshopt_compression"));
  assert.equal((await generatePreview(input)).bytes.length, bytes.length);
  assert.equal(await previewFingerprint(), await previewFingerprint());
});

test("previews drop reveal-only nodes and keep covered nodes and empty part nodes", async (t) => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "preview-reveal-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const doc = new Document();
  const buffer = doc.createBuffer();
  const mesh = () => {
    const positions = doc
      .createAccessor()
      .setType("VEC3")
      .setArray(new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0]))
      .setBuffer(buffer);
    return doc.createMesh().addPrimitive(doc.createPrimitive().setAttribute("POSITION", positions));
  };
  const part = doc.createNode("building-001");
  part.addChild(
    doc
      .createNode("covered")
      .setMesh(mesh())
      .setExtras({ reveal_hide_when_applied: ["patch-000"] }),
  );
  part.addChild(
    doc
      .createNode("revealed")
      .setMesh(mesh())
      .setExtras({ reveal_show_when_applied: ["patch-000"] }),
  );
  doc
    .createScene()
    .addChild(doc.createNode("map").addChild(part))
    .addChild(doc.createNode("empty-part"));
  const input = path.join(root, "model.glb");
  await new NodeIO().write(input, doc);
  const { bytes } = await generatePreview(input);
  const result = await new NodeIO()
    .registerExtensions((await import("@gltf-transform/extensions")).ALL_EXTENSIONS)
    .registerDependencies({ "meshopt.decoder": (await import("meshoptimizer")).MeshoptDecoder })
    .readBinary(meshoptReadable(bytes));
  const names = result
    .getRoot()
    .listNodes()
    .map((node) => node.getName())
    .sort();
  assert.deepEqual(names, ["building-001", "covered", "empty-part", "map"]);
  assert.equal(result.getRoot().listMeshes().length, 1);
});
