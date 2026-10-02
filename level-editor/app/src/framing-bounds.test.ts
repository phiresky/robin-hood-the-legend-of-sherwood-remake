import test from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import { FramingBounds } from "./framing-bounds.ts";

test("framing uses eight frozen local-bound corners per placement regardless of tessellation", () => {
  const root = new THREE.Group();
  root.rotation.y = 0.3;
  const geometry = new THREE.BoxGeometry(5, 7, 9, 20, 20, 20);
  const attribute = geometry.getAttribute("position");
  for (let i = 0; i < 100; i++) {
    const mesh = new THREE.Mesh(geometry);
    mesh.position.set(i * 3, i % 5, -i);
    mesh.scale.set(1 + i / 100, 2, 0.5);
    mesh.rotation.z = i / 10;
    root.add(mesh);
  }
  root.updateWorldMatrix(true, true);
  const expected: number[][] = [];
  for (const child of root.children)
    for (const x of [-2.5, 2.5])
      for (const y of [-3.5, 3.5])
        for (const z of [-4.5, 4.5])
          expected.push(new THREE.Vector3(x, y, z).applyMatrix4(child.matrixWorld).toArray());
  const snapshot = new FramingBounds([root]);
  assert.equal(snapshot.length, 800);
  assert.deepEqual(
    Array.from(snapshot, (p) => p.toArray()),
    expected,
  );
  // Editing must not move saved framing until the viewport explicitly rebuilds it.
  attribute.setXYZ(0, 500, 600, 700);
  root.position.set(100, 200, 300);
  root.updateWorldMatrix(true, true);
  assert.deepEqual(
    Array.from(snapshot, (p) => p.toArray()),
    expected,
  );
  geometry.dispose();
});

test("framing bounds decode normalized interleaved positions and skip hidden or empty meshes", () => {
  const geometry = new THREE.BufferGeometry();
  const buffer = new THREE.InterleavedBuffer(
    new Int16Array([123, 32767, 0, -32767, 456, 0, 16384, 32767]),
    4,
  );
  geometry.setAttribute("position", new THREE.InterleavedBufferAttribute(buffer, 3, 1, true));
  const mesh = new THREE.Mesh(geometry);
  const hidden = new THREE.Group();
  hidden.visible = false;
  hidden.add(new THREE.Mesh(geometry));
  const root = new THREE.Group();
  root.add(mesh, hidden, new THREE.Mesh(new THREE.BufferGeometry()));
  const snapshot = new FramingBounds([root]);
  assert.equal(snapshot.length, 8);
  const box = new THREE.Box3();
  for (const point of snapshot) box.expandByPoint(point);
  assert.deepEqual(box.min.toArray(), [0, 0, -1]);
  assert.deepEqual(box.max.toArray(), [1, 16384 / 32767, 1]);
  assert.equal(new FramingBounds().length, 0);
  geometry.dispose();
});
