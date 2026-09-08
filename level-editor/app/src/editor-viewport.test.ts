import test from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import { EditorViewport } from "./editor-viewport.ts";

test("map owner rejects replacement without retirement and releases detached ground exactly once", () => {
  const viewport = new EditorViewport();
  const asset = new THREE.Group();
  const ground = new THREE.Mesh(
    new THREE.BoxGeometry(),
    new THREE.MeshBasicMaterial(),
  );
  asset.add(ground);
  const parent = new THREE.Group();
  const overlay = new THREE.Group();
  let disposals = 0;
  ground.geometry.addEventListener("dispose", () => disposals++);
  viewport.installMap(asset, ground, parent);
  assert.equal(ground.parent, parent);
  assert.throws(
    () => viewport.installMap(new THREE.Group(), null, parent),
    /retiring/,
  );
  viewport.retireMap(overlay);
  assert.equal(ground.parent, null);
  assert.equal(viewport.groundNode, null);
  assert.equal(disposals, 1);
  viewport.retireMap(overlay);
  assert.equal(disposals, 1);
  viewport.installMap(new THREE.Group(), null, parent);
});
