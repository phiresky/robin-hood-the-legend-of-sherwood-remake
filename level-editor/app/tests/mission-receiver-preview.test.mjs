import test from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import { MissionEntities } from "../src/mission.ts";
const receiver = [
  {
    id: "a",
    points: [
      [-10, -10, 0],
      [10, -10, 0],
      [-10, 10, 0],
    ],
  },
  {
    id: "b",
    points: [
      [10, -10, 0],
      [10, 10, 0],
      [-10, 10, 0],
    ],
  },
];
const frame = (id) => ({
  geometry: new THREE.PlaneGeometry(2, 2),
  texture: new THREE.Texture(),
  shadow: new THREE.Texture(),
  bounds: { left: 0, top: 0, width: 2, height: 2 },
  shadowBounds: { left: 0, top: 0, width: 2, height: 2 },
  source: { resourceId: id },
});
test("editable receiver preview updates the selected body and shadow direction and retires stale support", () => {
  const a = frame(1),
    b = frame(2),
    body = new THREE.Mesh(a.geometry, new THREE.MeshBasicMaterial({ map: a.texture })),
    view = new MissionEntities();
  view.root.add(body);
  view.actors = [
    {
      mesh: body,
      frames: new Map([
        [0, a],
        [4, b],
      ]),
      direction: 0,
      shadow: null,
    },
  ];
  view.bindCharacterReceivers("soldiers:0", "bank-revision-1", receiver, Math.PI / 4);
  assert.equal(view.characterReceiverStatus().ready, true);
  assert.equal(body.children[0].material.map, a.shadow);
  const camera = new THREE.OrthographicCamera(-10, 10, 10, -10, 0.1, 1000);
  camera.position.set(0, 10, 100);
  camera.lookAt(0, 0, 0);
  view.actors[0].direction = 4;
  view.update(camera);
  assert.equal(body.geometry, b.geometry);
  assert.equal(body.material.map, b.texture);
  assert.equal(body.children[0].material.map, b.shadow);
  const shadow = body.children[0];
  view.update(camera);
  assert.equal(body.children[0], shadow, "unchanged geometry is retained");
  body.position.x = 100;
  view.update(camera);
  assert.equal(view.characterReceiverStatus().ready, false);
  assert.match(view.characterReceiverStatus().error, /coverage|support/i);
  assert.equal(body.children.length, 0);
  view.dispose();
  for (const f of [a, b]) {
    f.geometry.dispose();
    f.texture.dispose();
    f.shadow.dispose();
  }
  body.material.dispose();
});
test("failed explicit support replacement preserves the preceding binding; hidden actors drop shadows", () => {
  const a = frame(1),
    body = new THREE.Mesh(a.geometry, new THREE.MeshBasicMaterial({ map: a.texture })),
    view = new MissionEntities();
  view.root.add(body);
  view.actors = [{ mesh: body, frames: new Map([[0, a]]), direction: 0, shadow: null }];
  view.bindCharacterReceivers("soldiers:0", "r1", receiver, Math.PI / 4);
  const shadow = body.children[0];
  assert.throws(
    () => view.bindCharacterReceivers("soldiers:0", "bad", [], Math.PI / 4),
    /Missing support/,
  );
  assert.equal(body.children[0], shadow);
  assert.equal(view.characterReceiverStatus().revision, "r1");
  body.visible = false;
  const camera = new THREE.OrthographicCamera(-10, 10, 10, -10, 0.1, 1000);
  camera.position.z = 100;
  camera.lookAt(0, 0, 0);
  view.update(camera);
  assert.equal(body.children.length, 0);
  view.dispose();
  a.geometry.dispose();
  a.texture.dispose();
  a.shadow.dispose();
  body.material.dispose();
});
