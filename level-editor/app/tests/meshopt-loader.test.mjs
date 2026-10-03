import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { createGltfLoader } from "../src/gltf-loader.ts";
import { compressGlb } from "../../pipeline/src/meshopt-glb.ts";
const require = createRequire(new URL("../../pipeline/package.json", import.meta.url));
const { Document, NodeIO } = await import(require.resolve("@gltf-transform/core"));

test("editor GLTFLoader decodes KHR meshopt v1 with unchanged positions and indices", async () => {
  const doc = new Document(),
    buffer = doc.createBuffer();
  const positions = new Float32Array(9000),
    indices = new Uint16Array(3000);
  for (let i = 0; i < 3000; i++) {
    positions[i * 3] = i % 40;
    positions[i * 3 + 1] = Math.floor(i / 40);
    indices[i] = i;
  }
  const primitive = doc
    .createPrimitive()
    .setAttribute(
      "POSITION",
      doc.createAccessor().setType("VEC3").setArray(positions).setBuffer(buffer),
    )
    .setIndices(doc.createAccessor().setType("SCALAR").setArray(indices).setBuffer(buffer));
  doc
    .createScene()
    .addChild(doc.createNode("mesh").setMesh(doc.createMesh().addPrimitive(primitive)));
  const original = await new NodeIO().writeBinary(doc),
    compressed = await compressGlb(original);
  assert.ok(compressed.length < original.length);
  const gltf = await createGltfLoader().parseAsync(new Uint8Array(compressed).buffer, "");
  const mesh = gltf.scene.getObjectByName("mesh");
  assert.deepEqual(mesh.geometry.attributes.position.array, positions);
  assert.deepEqual(mesh.geometry.index.array, indices);
  mesh.geometry.dispose();
  mesh.material.dispose();
});
