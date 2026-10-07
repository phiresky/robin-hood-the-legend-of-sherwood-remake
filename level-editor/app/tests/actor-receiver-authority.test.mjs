import test from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import { assetNodeKey } from "../../shared/src/index.ts";
import { currentReceiverMeshes } from "../src/actor-receiver-authority.ts";
const authority = {
  asset: "bank",
  model_sha256: "a".repeat(64),
  physicalParts: [{ node: "surface", meshes: ["physical top"] }],
};
const document = () => ({
  map: "test",
  camera: { kind: "oblique-orthographic", elevation_deg: 35 },
  assetSources: [{ id: "bank", model_sha256: "a".repeat(64) }],
  groups: [{ id: "placed-bank", name: "Bank" }],
  objects: [{ id: "part", group: "placed-bank", node: assetNodeKey("bank", "surface") }],
});
const make = () => {
  const mesh = new THREE.Mesh(new THREE.PlaneGeometry(10, 10));
  mesh.rotation.x = -Math.PI / 2;
  mesh.name = "physical top";
  return mesh;
};
test("explicit physical mesh authority excludes same-part contact art and follows current transforms", () => {
  const mesh = make(),
    appearance = make();
  appearance.name = "contact appearance";
  const d = document(),
    lookup = () => [appearance, mesh];
  const a = currentReceiverMeshes(d, "placed-bank", authority, lookup);
  assert.equal(a.evaluate().length, 2);
  assert.ok(a.evaluate().every((t) => t.id.includes("physical top")));
  mesh.position.y = 20;
  const b = currentReceiverMeshes(d, "placed-bank", authority, lookup);
  assert.notEqual(a.revision, b.revision);
  assert.ok(
    b
      .evaluate()
      .every((t) =>
        t.points.every((p) => Math.abs(p[2] - 20 * Math.cos((35 * Math.PI) / 180)) < 1e-6),
      ),
  );
  mesh.geometry.dispose();
  appearance.geometry.dispose();
  mesh.material.dispose();
  appearance.material.dispose();
});
test("changed models, hidden parts and duplicate physical identities are rejected", () => {
  const mesh = make(),
    d = document();
  d.assetSources[0].model_sha256 = "b".repeat(64);
  assert.throws(
    () => currentReceiverMeshes(d, "placed-bank", authority, () => [mesh]),
    /reviewed source/,
  );
  d.assetSources[0].model_sha256 = authority.model_sha256;
  d.objects[0].hidden = true;
  assert.throws(() => currentReceiverMeshes(d, "placed-bank", authority, () => [mesh]), /hidden/);
  d.objects[0].hidden = false;
  assert.throws(
    () => currentReceiverMeshes(d, "placed-bank", authority, () => [mesh, mesh]),
    /ambiguous/,
  );
  mesh.geometry.dispose();
  mesh.material.dispose();
});
