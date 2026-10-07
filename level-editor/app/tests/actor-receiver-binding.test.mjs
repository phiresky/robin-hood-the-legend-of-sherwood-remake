import test from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import { createReceiverActorFrameBinding } from "../src/actor-receiver-binding.ts";
const receiverTriangles = [
  [
    [-100, -100, 0],
    [100, -100, 0],
    [-100, 100, 0],
  ],
  [
    [100, -100, 0],
    [100, 100, 0],
    [-100, 100, 0],
  ],
];
const frame = (width = 10) => ({
  geometry: new THREE.PlaneGeometry(width, 10),
  texture: new THREE.Texture(),
  shadow: new THREE.Texture(),
  bounds: { left: -width / 2, top: 10, width, height: 10 },
});
function setup() {
  const initial = frame(),
    body = new THREE.Mesh(initial.geometry, new THREE.MeshBasicMaterial({ map: initial.texture })),
    binding = createReceiverActorFrameBinding(body, "soldiers:0");
  const snapshot = {
    identity: "soldiers:0",
    frame: initial,
    anchor: [0, 0, 0],
    rotation: 0,
    active: true,
    elevation: Math.PI / 4,
    receiverTriangles,
    shadowStyle: { color: 0, opacity: 0.4 },
  };
  binding.apply(snapshot);
  return { body, binding, snapshot };
}
test("receiver-backed directional replacement updates body, keyed shadow texture and bounds atomically", () => {
  const { body, binding, snapshot } = setup(),
    prior = binding.shadow,
    next = frame(20);
  let released = 0,
    borrowed = 0;
  prior.geometry.addEventListener("dispose", () => released++);
  prior.material.addEventListener("dispose", () => released++);
  snapshot.frame.shadow.addEventListener("dispose", () => borrowed++);
  snapshot.frame.geometry.addEventListener("dispose", () => borrowed++);
  binding.apply({ ...snapshot, frame: next, rotation: 0.7 });
  assert.equal(body.geometry, next.geometry);
  assert.equal(body.material.map, next.texture);
  assert.equal(binding.shadow.material.map, next.shadow);
  assert.equal(binding.shadow.rotation.y, -0.7);
  assert.equal(released, 2);
  assert.equal(borrowed, 0);
  const positions = binding.shadow.geometry.getAttribute("position");
  assert.equal(
    Math.min(...Array.from({ length: positions.count }, (_, i) => positions.getX(i))),
    -10,
  );
  assert.ok(binding.shadow.geometry.userData.receiverRanges.length);
  binding.dispose();
  assert.equal(body.children.length, 0);
  assert.equal(borrowed, 0);
});
test("missing receiver coverage cannot partially publish a new pose or frame", () => {
  const { body, binding, snapshot } = setup(),
    prior = binding.shadow,
    geometry = body.geometry,
    map = body.material.map;
  assert.throws(
    () =>
      binding.apply({
        ...snapshot,
        frame: frame(20),
        anchor: [80, 0, 0],
        receiverTriangles: [
          [
            [0, 0, 0],
            [1, 0, 0],
            [0, 1, 0],
          ],
        ],
      }),
    /Incomplete support/,
  );
  assert.equal(body.geometry, geometry);
  assert.equal(body.material.map, map);
  assert.equal(binding.shadow, prior);
  assert.deepEqual(body.position.toArray(), [0, 0, 0]);
  binding.dispose();
});
test("inactive or empty frames remove the receiver shadow while preserving borrowed images", () => {
  const { body, binding, snapshot } = setup();
  binding.apply({ ...snapshot, active: false });
  assert.equal(body.visible, false);
  assert.equal(binding.shadow, null);
  binding.apply(snapshot);
  binding.apply({ ...snapshot, frame: null });
  assert.equal(body.visible, false);
  assert.equal(body.children.length, 0);
  binding.dispose();
  binding.dispose();
  assert.throws(() => binding.apply(snapshot), /disposed/);
});
