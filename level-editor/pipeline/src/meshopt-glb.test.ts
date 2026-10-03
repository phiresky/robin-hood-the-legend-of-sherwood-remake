import test from "node:test";
import assert from "node:assert/strict";
import { Document, NodeIO } from "@gltf-transform/core";
import {
  compressGlb,
  decodeGlb,
  parseGlb,
  meshoptIO,
  meshoptReadable,
  upgradeMeshopt,
} from "./meshopt-glb.ts";
import { EXTMeshoptCompression, ALL_EXTENSIONS } from "@gltf-transform/extensions";
import { MeshoptEncoder } from "meshoptimizer";

async function fixture(count: number) {
  const doc = new Document(),
    buffer = doc.createBuffer();
  const positions = new Float32Array(count * 3),
    indices = new Uint16Array(count);
  for (let i = 0; i < count; i++) {
    positions[i * 3] = i % 20;
    positions[i * 3 + 1] = Math.floor(i / 20);
    indices[i] = i;
  }
  const primitive = doc
    .createPrimitive()
    .setAttribute(
      "POSITION",
      doc.createAccessor().setType("VEC3").setArray(positions).setBuffer(buffer),
    )
    .setIndices(doc.createAccessor().setType("SCALAR").setArray(indices).setBuffer(buffer));
  const texture = doc
    .createTexture()
    .setMimeType("image/png")
    .setImage(
      Buffer.from(
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aWQAAAABJRU5ErkJggg==",
        "base64",
      ),
    );
  primitive.setMaterial(doc.createMaterial().setBaseColorTexture(texture));
  doc.createScene().addChild(
    doc
      .createNode("tree")
      .setExtras({ reveal_show_when_applied: ["patch"], EXT_meshopt_compression: "user data" })
      .setMesh(doc.createMesh().addPrimitive(primitive)),
  );
  return { doc, bytes: await new NodeIO().writeBinary(doc) };
}

test("v1 preserves accessor bytes, metadata and texture bytes; loader can decode it", async () => {
  const { bytes } = await fixture(6000);
  const packed = await compressGlb(bytes);
  assert.ok(packed.length < bytes.length / 2);
  assert.deepEqual(await compressGlb(packed), packed);
  const original = parseGlb(bytes),
    encoded = parseGlb(packed),
    decoded = parseGlb(await decodeGlb(packed));
  assert.ok(encoded.json.extensionsRequired?.includes("KHR_meshopt_compression"));
  assert.deepEqual(decoded.json.nodes, original.json.nodes);
  assert.deepEqual(decoded.json.meshes, original.json.meshes);
  assert.deepEqual(decoded.json.accessors, original.json.accessors);
  for (const [i, before] of original.json.bufferViews!.entries()) {
    const after = decoded.json.bufferViews![i]!;
    assert.deepEqual(
      decoded.bin.subarray(after.byteOffset ?? 0, (after.byteOffset ?? 0) + after.byteLength),
      original.bin.subarray(before.byteOffset ?? 0, (before.byteOffset ?? 0) + before.byteLength),
    );
  }
  for (const view of encoded.json.bufferViews!) {
    const ext = view.extensions?.KHR_meshopt_compression as
      | { mode: string; byteOffset: number }
      | undefined;
    if (ext?.mode === "ATTRIBUTES") assert.equal(encoded.bin[ext.byteOffset], 0xa1);
  }
  const io = await meshoptIO();
  const loaded = await io.readBinary(meshoptReadable(packed));
  assert.deepEqual(
    loaded.getRoot().listMeshes()[0]!.listPrimitives()[0]!.getIndices()!.getArray(),
    new Uint16Array(Array.from({ length: 6000 }, (_, i) => i)),
  );
});

test("small geometry retains the exact input when encoding adds overhead", async () => {
  const { bytes } = await fixture(3);
  assert.deepEqual(await compressGlb(bytes), bytes);
});

test("EXT preview upgrade emits v1 and preserves decoded streams", async () => {
  const { doc } = await fixture(600);
  await MeshoptEncoder.ready;
  doc
    .createExtension(EXTMeshoptCompression)
    .setRequired(true)
    .setEncoderOptions({ method: EXTMeshoptCompression.EncoderMethod.FILTER });
  const io = new NodeIO()
    .registerExtensions(ALL_EXTENSIONS)
    .registerDependencies({ "meshopt.encoder": MeshoptEncoder });
  const before = await io.writeBinary(doc),
    after = await upgradeMeshopt(before);
  assert.ok(parseGlb(after).json.extensionsRequired?.includes("KHR_meshopt_compression"));
  assert.deepEqual(await decodeGlb(after), await decodeGlb(before));
});
