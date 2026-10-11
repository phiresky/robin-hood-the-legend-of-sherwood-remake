import test from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import { ReviewedNodePoses, validateReviewedNodePoses } from "./reviewed-node-poses.ts";

const rows = (matrix: THREE.Matrix4) => [{ node: "link", matrix: matrix.toArray() }];
test("absolute poses preserve coordinate wrapper and restore original mesh transforms", () => {
  const root = new THREE.Group();
  root.position.set(200, 100, -40);
  root.rotation.x = -Math.PI / 2;
  const mesh = new THREE.Mesh();
  mesh.name = "link";
  mesh.position.set(3, 2, 1);
  mesh.rotation.z = 0.3;
  root.add(mesh);
  root.updateMatrixWorld(true);
  const original = mesh.matrix.clone(),
    world = mesh.matrixWorld.clone();
  const delta = new THREE.Matrix4().makeRotationZ(0.9).setPosition(5, 2, 0);
  const controller = new ReviewedNodePoses(root, [rows(delta), rows(new THREE.Matrix4())]);
  for (let i = 0; i < 5; i++) controller.apply(0);
  root.updateMatrixWorld(true);
  const expected = root.matrixWorld.clone().multiply(delta).multiply(original);
  assert.deepEqual(mesh.matrixWorld.toArray(), expected.toArray());
  controller.apply(1);
  root.updateMatrixWorld(true);
  assert.deepEqual(mesh.matrixWorld.toArray(), world.toArray());
  controller.reset();
  assert.equal(mesh.matrixAutoUpdate, true);
  assert.deepEqual(mesh.matrix.toArray(), original.toArray());
});

test("invalid bindings and deformation fail before any object mutation", () => {
  const root = new THREE.Group(),
    mesh = new THREE.Mesh();
  mesh.name = "link";
  root.add(mesh);
  const before = mesh.matrix.toArray();
  const table = [
    rows(new THREE.Matrix4()),
    [{ node: "absent", matrix: new THREE.Matrix4().toArray() }],
  ];
  assert.throws(() => new ReviewedNodePoses(root, table), /identities/);
  assert.deepEqual(mesh.matrix.toArray(), before);
  assert.equal(mesh.matrixAutoUpdate, true);
  assert.throws(
    () => validateReviewedNodePoses([rows(new THREE.Matrix4().makeScale(2, 1, 1))]),
    /rigid/,
  );
  root.add(mesh.clone());
  assert.throws(() => new ReviewedNodePoses(root, [rows(new THREE.Matrix4())]), /ambiguous/);
});
