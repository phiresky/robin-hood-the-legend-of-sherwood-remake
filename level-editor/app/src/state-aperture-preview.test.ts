import test from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import { StateAperturePreview } from "./state-aperture-preview.ts";
function fixture() {
  const scene = new THREE.Scene(),
    root = new THREE.Group(),
    receiver = new THREE.Mesh(new THREE.PlaneGeometry(), new THREE.MeshBasicMaterial());
  scene.add(root, receiver);
  const make = (id: string) => {
    const cap = new THREE.Group(),
      endpoint = new THREE.Group(),
      initial = new THREE.Group(),
      applied = new THREE.Group();
    cap.userData.reveal_hide_when_applied = [id];
    initial.userData.reveal_hide_when_applied = [id];
    applied.userData.reveal_show_when_applied = [id];
    endpoint.add(initial, applied);
    root.add(cap, endpoint);
    return { id, cap, endpoint, initial, applied };
  };
  const a = make("a"),
    b = make("b"),
    c = make("c");
  const controller = new StateAperturePreview({
    root,
    originalReceivers: [receiver],
    bindings: [
      { family: "first", mission: "one", patches: ["a"], endpointParent: a.endpoint },
      { family: "second", mission: "one", patches: ["b"], endpointParent: b.endpoint },
      { family: "first", mission: "two", patches: ["c"], endpointParent: c.endpoint },
    ],
  });
  return { scene, root, receiver, a, b, c, controller };
}
test("independent apertures preserve source geometry and reset only the selected family", () => {
  const f = fixture(),
    positions = Array.from(f.receiver.geometry.getAttribute("position").array);
  f.controller.selectMission("one");
  f.controller.selectEnabled(true);
  f.controller.selectEndpoint("first", "applied");
  f.controller.selectEndpoint("second", "applied");
  assert(!f.a.cap.visible && !f.b.cap.visible && !f.receiver.visible);
  f.controller.selectEndpoint("first", "initial");
  assert(f.a.cap.visible && !f.b.cap.visible && f.b.applied.visible);
  assert.deepEqual(Array.from(f.receiver.geometry.getAttribute("position").array), positions);
  f.controller.dispose();
  assert(f.receiver.visible);
  assert.equal(f.root.parent, null);
});
test("mission switch resets all caps and limits physical endpoint parents", () => {
  const f = fixture();
  f.controller.selectMission("one");
  f.controller.selectEnabled(true);
  f.controller.selectEndpoint("first", "applied");
  f.controller.selectMission("two");
  assert(f.a.cap.visible && f.b.cap.visible && f.c.cap.visible);
  assert(!f.a.endpoint.visible && !f.b.endpoint.visible && f.c.endpoint.visible);
  assert.throws(() => f.controller.selectEndpoint("second", "applied"), /Unknown/);
  f.controller.selectEndpoint("first", "applied");
  assert(!f.c.cap.visible && f.a.cap.visible);
  f.controller.dispose();
});
test("native mode and retirement restore a user-hidden receiver exactly", () => {
  const f = fixture();
  f.controller.dispose();
  f.receiver.visible = false;
  const controller = new StateAperturePreview({
    root: f.root,
    originalReceivers: [f.receiver],
    bindings: [{ family: "first", mission: "one", patches: ["a"], endpointParent: f.a.endpoint }],
  });
  controller.selectMission("one");
  controller.selectEnabled(true);
  controller.selectEndpoint("first", "applied");
  controller.selectEnabled(false);
  assert(!f.receiver.visible && !f.root.visible);
  controller.dispose();
  controller.dispose();
  assert(!f.receiver.visible);
});
test("moved receiver invalidates physical preview and disposal still restores visibility", () => {
  const f = fixture();
  f.controller.selectMission("one");
  f.receiver.position.x = 1;
  assert.throws(() => f.controller.selectEnabled(true), /moved/);
  f.controller.dispose();
  assert(f.receiver.visible);
});
