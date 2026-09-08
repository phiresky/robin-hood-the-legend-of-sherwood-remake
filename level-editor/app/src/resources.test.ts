import test from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import { disposeObjectResources } from "./resources.ts";

test("shared source geometry, materials and all texture slots dispose once", () => {
  const geometry = new THREE.BoxGeometry();
  const texture = new THREE.Texture();
  const material = new THREE.MeshStandardMaterial({
    map: texture,
    normalMap: texture,
  });
  const original = new THREE.Mesh(geometry, material);
  const clone = original.clone();
  const counts = [0, 0, 0];
  [geometry, material, texture].forEach((resource, i) =>
    resource.addEventListener("dispose", () => counts[i]!++),
  );
  disposeObjectResources([original, clone]);
  assert.deepEqual(counts, [1, 1, 1]);
});
