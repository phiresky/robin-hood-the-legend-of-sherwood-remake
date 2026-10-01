import test from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import { parseLevel3D, type Level3D } from "@rle/shared";
import { SunLighting } from "./sun-lighting.ts";

const settings = { enabled: true, sunAzimuth: 90, sunElevation: 45, shadowOpacity: 0.5 };
test("sun direction, caster exclusions, and borrowed terrain resource ownership", () => {
  const lighting = new SunLighting();
  const ground = new THREE.Mesh(new THREE.PlaneGeometry(100, 100), new THREE.MeshBasicMaterial());
  let disposed = 0;
  ground.geometry.addEventListener("dispose", () => disposed++);
  ground.material.addEventListener("dispose", () => disposed++);
  lighting.setGround(ground);
  const caster = new THREE.Mesh(new THREE.BoxGeometry(), new THREE.MeshBasicMaterial());
  const water = caster.clone();
  water.userData.noSunShadow = true;
  lighting.sync(
    settings,
    [caster, water],
    new THREE.Box3(new THREE.Vector3(-50, -50, 0), new THREE.Vector3(50, 50, 20)),
  );
  const direction = lighting.sun.position.clone().sub(lighting.sun.target.position).normalize();
  assert.ok(
    Math.abs(direction.x - Math.SQRT1_2) < 1e-6 && Math.abs(direction.z - Math.SQRT1_2) < 1e-6,
  );
  assert.equal(caster.castShadow, true);
  assert.equal(water.castShadow, false);
  assert.equal(lighting.sun.shadow.intensity, settings.shadowOpacity);
  const receiver = lighting.root.children.find((node) => node instanceof THREE.Mesh) as THREE.Mesh;
  assert.equal((receiver.material as THREE.ShadowMaterial).opacity, 1);
  lighting.sync(undefined, [caster, water], new THREE.Box3());
  assert.equal(lighting.sun.visible, false);
  assert.equal(receiver.visible, false);
  assert.equal(lighting.ambient.intensity, Math.PI);
  lighting.setGround(null);
  lighting.dispose();
  assert.equal(disposed, 0);
  caster.geometry.dispose();
  caster.material.dispose();
  ground.geometry.dispose();
  ground.material.dispose();
});
test("lighting settings round-trip and reject out-of-range solar controls", () => {
  const document: Level3D = {
    version: 1,
    map: "Test",
    sceneAssets: [],
    size: [100, 100],
    camera: { kind: "oblique-orthographic", elevation_deg: 35 },
    objects: [],
    groups: [],
    lighting: settings,
  };
  assert.deepEqual(parseLevel3D(JSON.parse(JSON.stringify(document))).lighting, settings);
  for (const invalid of [{ sunElevation: 0 }, { sunAzimuth: 361 }, { shadowOpacity: 2 }])
    assert.throws(
      () => parseLevel3D({ ...document, lighting: { ...settings, ...invalid } }),
      /lighting/,
    );
});
