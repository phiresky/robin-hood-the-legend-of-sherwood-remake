import test from "node:test";
import assert from "node:assert/strict";
import { NativeActorComposition, type CurrentActorSnapshot } from "./native-actor-composition.ts";
import type { NativeLoopDrawSnapshot } from "./native-state-presentation.ts";
const pixels = { width: 1, height: 1, data: new Uint8Array([255, 0, 0, 255]) };
const loop: NativeLoopDrawSnapshot = {
  revision: 1,
  mission: "S03_FoB_MP",
  origin: [0, 0],
  background: pixels,
  draws: [
    {
      element: {
        id: "sign4",
        source: { kind: "mission-target", index: 4, sha256: "0".repeat(64) },
        active: true,
        frames: [],
        loop: false,
        display_position: [0, 0],
        sort_position: [0, 200],
        display_order: 200,
        creation_order: 19,
        polyline: [],
      },
      frame: 0,
      pixels,
      x: 0,
      y: 0,
    },
  ],
};
const snapshot: CurrentActorSnapshot = {
  epoch: 3,
  mission: "S03_FoB_MP",
  tick: 0,
  actors: [
    {
      identity: "soldiers:0",
      active: true,
      displayOrder: 200,
      sprite: {
        localIndex: 0,
        position: [0, 0, 0],
        direction: 0,
        visible: true,
        bounds: { left: 0, top: 0, width: 1, height: 1 },
        frame: {
          resourceId: 1,
          filename: "fixture",
          profile: "fixture",
          action: 3,
          direction: 0,
          frame: 0,
          legacy: true,
          pixels,
        },
      },
      masks: [],
      maskQuery: {
        layer: 0,
        mapPosition: [0, 0],
        screenOrigin: [0, 0],
        drawHidden: false,
        outlineColor: 0xf800,
        depth: 16,
        shadowKey: 31,
        shadowStrength: 40,
      },
    },
  ],
};
const create = () =>
  new NativeActorComposition({
    epoch: 3,
    mission: "S03_FoB_MP",
    creationRanks: new Map([
      ["soldiers:0", 40],
      ["mission-target:4", 104],
    ]),
    backgroundEffects: new Set(),
  });
test("actor merge uses full construction ranks when local effect ranks give the opposite order", () => {
  const composition = create(),
    prepared = composition.prepare(loop, snapshot);
  assert.ok(loop.draws[0]!.element.creation_order < 40);
  assert.deepEqual(prepared.identities, ["soldiers:0", "mission-target:4"]);
  const moved = structuredClone(snapshot);
  moved.actors[0]!.displayOrder = 201;
  assert.deepEqual(composition.prepare(loop, moved).identities, ["mission-target:4", "soldiers:0"]);
  composition.dispose();
});
test("retired snapshots, duplicate ownership and missing ranks fail before any rendering", () => {
  const composition = create();
  assert.throws(() => composition.prepare(loop, { ...snapshot, epoch: 2 }), /retired/);
  assert.throws(
    () =>
      composition.prepare(loop, { ...snapshot, actors: [...snapshot.actors, ...snapshot.actors] }),
    /Duplicate/,
  );
  assert.throws(
    () =>
      composition.prepare(loop, {
        ...snapshot,
        actors: [{ ...snapshot.actors[0]!, identity: "unknown" }],
      }),
    /Unresolved/,
  );
  composition.dispose();
  assert.throws(() => composition.prepare(loop, snapshot), /disposed/);
});
test("inactive actors retain validated identities but do not draw and snapshots do not advance", () => {
  const composition = create(),
    inactive = { ...snapshot, actors: [{ ...snapshot.actors[0]!, active: false }] };
  assert.deepEqual(composition.prepare(loop, inactive).identities, ["mission-target:4"]);
  assert.deepEqual(composition.prepare(loop, snapshot), composition.prepare(loop, snapshot));
  assert.equal(snapshot.tick, 0);
  assert.equal(loop.revision, 1);
  composition.dispose();
});
