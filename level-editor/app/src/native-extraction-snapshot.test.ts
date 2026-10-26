import test from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import { createNativeExtractionGeometrySnapshot } from "./native-extraction-snapshot.ts";

test("session geometry snapshots share copied backing across distinct accessor views and isolate source writes", () => {
  const storage = new ArrayBuffer(128);
  const positions = new THREE.BufferAttribute(new Float32Array(storage, 0, 9), 3);
  positions.array.set([0, 0, 0, 1, 0, 0, 0, 1, 0]);
  const normals = new THREE.BufferAttribute(new Int8Array(storage, 40, 9), 3, true);
  normals.array.set([0, 0, 127, 0, 0, 127, 0, 0, 127]);
  const uv = new THREE.BufferAttribute(new Float32Array(storage, 52, 6), 2);
  uv.array.set([0, 0, 1, 0, 0, 1]);
  const index = new THREE.BufferAttribute(new Uint16Array(storage, 80, 3), 1);
  index.array.set([0, 1, 2]);
  const morph = new THREE.BufferAttribute(new Float32Array(storage, 88, 9), 3);
  morph.array.set([0, 0, 1, 0, 0, 1, 0, 0, 1]);
  const make = () => {
    const g = new THREE.BufferGeometry();
    g.setAttribute("position", positions);
    g.setAttribute("normal", normals);
    g.setAttribute("uv", uv);
    g.setIndex(index);
    g.morphAttributes.position = [morph];
    g.morphTargetsRelative = true;
    g.addGroup(0, 3, 2);
    g.setDrawRange(0, 3);
    g.userData = { ownership: { role: "owner" } };
    return g;
  };
  const clone = createNativeExtractionGeometrySnapshot(),
    sourceA = make(),
    sourceB = make();
  const a = clone(sourceA),
    b = clone(sourceB);
  assert.notEqual(a, b);
  assert.equal(a.attributes.position, b.attributes.position);
  assert.equal(a.index, b.index);
  const attrs = [
    a.attributes.position!,
    a.attributes.normal!,
    a.attributes.uv!,
    a.index!,
    a.morphAttributes.position![0]!,
  ];
  assert.equal(new Set(attrs.map((x) => x.array.buffer)).size, 1);
  assert.equal(attrs[0]!.array.buffer.byteLength, 128);
  assert.notEqual(attrs[0]!.array.buffer, storage);
  assert.equal(a.attributes.normal!.normalized, true);
  assert.equal(a.attributes.uv!.array.byteOffset, 52);
  assert.deepEqual(Array.from(a.index!.array), [0, 1, 2]);
  assert.deepEqual(a.groups, sourceA.groups);
  assert.deepEqual(a.drawRange, sourceA.drawRange);
  assert.equal(a.morphTargetsRelative, true);
  const expected = attrs.map((x) => Array.from(x.array));
  new Uint8Array(storage).fill(17);
  sourceA.userData.ownership.role = "changed";
  assert.deepEqual(
    attrs.map((x) => Array.from(x.array)),
    expected,
  );
  assert.equal(a.userData.ownership.role, "owner");
  let sourceDisposed = 0;
  sourceA.addEventListener("dispose", () => sourceDisposed++);
  a.dispose();
  b.dispose();
  assert.equal(sourceDisposed, 0);
});

test("independent sessions do not share copied backing or caller disposal ownership", () => {
  const source = new THREE.PlaneGeometry(2, 2);
  const a = createNativeExtractionGeometrySnapshot()(source);
  const b = createNativeExtractionGeometrySnapshot()(source);
  assert.notEqual(a.attributes.position!.array.buffer, b.attributes.position!.array.buffer);
  const before = Array.from(a.attributes.position!.array);
  source.dispose();
  source.attributes.position!.array.fill(0);
  assert.deepEqual(Array.from(a.attributes.position!.array), before);
  assert.deepEqual(Array.from(b.attributes.position!.array), before);
});

test("interleaved attribute snapshots preserve shared stride, offsets and normalized semantics", () => {
  const data = new THREE.InterleavedBuffer(
    new Float32Array([1, 2, 3, 0.25, 0.5, 4, 5, 6, 0.75, 1]),
    5,
  );
  data.setUsage(THREE.DynamicDrawUsage);
  const p = new THREE.InterleavedBufferAttribute(data, 3, 0);
  const uv = new THREE.InterleavedBufferAttribute(data, 2, 3, true);
  const g = new THREE.BufferGeometry().setAttribute("position", p).setAttribute("uv", uv);
  const copy = createNativeExtractionGeometrySnapshot()(g);
  const cp = copy.attributes.position as THREE.InterleavedBufferAttribute;
  const cuv = copy.attributes.uv as THREE.InterleavedBufferAttribute;
  assert.equal(cp.data, cuv.data);
  assert.notEqual(cp.data, data);
  assert.equal(cp.data.stride, 5);
  assert.equal(cuv.offset, 3);
  assert.equal(cuv.normalized, true);
  assert.equal(cp.data.usage, THREE.DynamicDrawUsage);
  assert.equal(cp.getZ(1), 6);
  assert.equal(cuv.getX(1), 0.75);
  data.array.fill(0);
  assert.equal(cp.getZ(1), 6);
  assert.equal(cuv.getX(1), 0.75);
});

test("half-float accessor interpretation and GPU type survive snapshot copying", () => {
  const p = new THREE.Float16BufferAttribute(new Uint16Array([0x3c00, 0x4000, 0x4200]), 3);
  p.gpuType = THREE.IntType;
  const copy = createNativeExtractionGeometrySnapshot()(
    new THREE.BufferGeometry().setAttribute("position", p),
  );
  assert.equal(copy.attributes.position!.getX(0), 1);
  assert.equal(copy.attributes.position!.getY(0), 2);
  assert.equal((copy.attributes.position as THREE.BufferAttribute).gpuType, THREE.IntType);
  assert.deepEqual(Array.from(copy.attributes.position!.array), [0x3c00, 0x4000, 0x4200]);
});
