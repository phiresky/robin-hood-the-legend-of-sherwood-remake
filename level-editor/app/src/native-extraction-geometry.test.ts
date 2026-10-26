import test from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import { verifyNativeExtractionGeometry } from "./native-extraction-geometry.ts";

const normalizedCases = [
  { array: new Uint8Array([1, 127, 255]), componentType: 5121 },
  { array: new Int8Array([-128, 1, 127]), componentType: 5120 },
  { array: new Uint16Array([1, 32767, 65535]), componentType: 5123 },
  { array: new Int16Array([-32768, 1, 32767]), componentType: 5122 },
  { array: new Uint32Array([1, 16777217, 4294967295]), componentType: 5125 },
];

for (const { array, componentType } of normalizedCases) {
  test(`normalized ${array.constructor.name} retains exact precision and rejects mutation`, () => {
    const raw = new Uint8Array(array.buffer);
    const binaryLength = (raw.length + 3) & ~3;
    const json = {
      accessors: [{ bufferView: 0, componentType, count: 1, type: "VEC3", normalized: true }],
      bufferViews: [{ buffer: 0, byteOffset: 0, byteLength: raw.length }],
      meshes: [{ primitives: [{ attributes: { COLOR_0: 0 } }] }],
    };
    const encoded = Buffer.from(JSON.stringify(json));
    const jsonChunk = Buffer.concat([encoded, Buffer.alloc((4 - (encoded.length % 4)) % 4, 32)]);
    const bytes = new Uint8Array(28 + jsonChunk.length + binaryLength);
    const header = new DataView(bytes.buffer);
    header.setUint32(0, 0x46546c67, true);
    header.setUint32(4, 2, true);
    header.setUint32(8, bytes.length, true);
    header.setUint32(12, jsonChunk.length, true);
    header.setUint32(16, 0x4e4f534a, true);
    bytes.set(jsonChunk, 20);
    header.setUint32(20 + jsonChunk.length, binaryLength, true);
    header.setUint32(24 + jsonChunk.length, 0x004e4942, true);
    bytes.set(raw, 28 + jsonChunk.length);

    const geometry = new THREE.BufferGeometry();
    geometry.setAttribute("color", new THREE.BufferAttribute(array, 3, true));
    const material = new THREE.MeshBasicMaterial();
    const mesh = new THREE.Mesh(geometry, material);
    try {
      const check = verifyNativeExtractionGeometry(bytes, json, mesh, 0, 0);
      // These normalized fractions cannot all be represented exactly by Float32.
      check();
      // Caller byte and metadata aliases are not the pinned comparison authority.
      bytes.fill(0, 28 + jsonChunk.length);
      json.accessors[0]!.normalized = false;
      json.accessors[0]!.componentType = 5126;
      json.bufferViews[0]!.byteOffset = 99999;
      check();
      array[1] = 0;
      assert.throws(check, /immutable accessor changed/);
    } finally {
      geometry.dispose();
      material.dispose();
    }
  });
}
