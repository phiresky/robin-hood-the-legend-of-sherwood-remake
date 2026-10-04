import test from "node:test";
import assert from "node:assert/strict";
import { authorLinkedAppearance } from "./author-linked-appearance.ts";
import { joinedTransitionCompilerFixture } from "../../shared/test-fixtures/asset-gameplay.ts";
import { compileAssetGameplay } from "../../shared/src/compile-asset-gameplay.ts";

test("shared appearance authoring retains local effects and connects only matching placed controls", () => {
  const { hut, document, assets } = joinedTransitionCompilerFixture();
  const wing = assets.get("wing")!;
  wing.gameplay!.movementTransitions = [];
  const before = structuredClone([hut, wing]);
  const primary = hut.gameplay!.movementTransitions![0]!;
  const authored = authorLinkedAppearance(hut, wing, {
    transition: primary.id,
    appearance: "roof",
    node: "building-999",
    id: "roof-control",
    key: primary.join!.key,
    rebase: ([x, y, z]) => [x - 500, y, z],
  });
  assert.deepEqual([hut, wing], before);
  assert.deepEqual(authored.controller.movementTransitions![0]!.initialSight, primary.initialSight);
  assert.equal(authored.follower.movementTransitions![0]!.initialSight, undefined);
  assert.equal(authored.follower.movementTransitions![0]!.doorLinks, undefined);
  hut.gameplay = authored.controller;
  wing.gameplay = authored.follower;
  const joined = compileAssetGameplay(document, assets, [0, 0, 2000, 2000]);
  assert.equal(joined.movement_transitions!.length, 1);
  assert.equal(joined.movement_transitions![0]!.aliases!.length, 1);
  assert.equal(joined.movement_transitions![0]!.has_appearance, true);
  document.objects.find((part) => part.group === "wing")!.transform.dx += 1;
  const detached = compileAssetGameplay(document, assets, [0, 0, 2000, 2000]);
  assert.equal(detached.movement_transitions!.length, 2);
});

test("shared appearance authoring rejects conflicting controllers without modifying definitions", () => {
  const { hut, assets } = joinedTransitionCompilerFixture();
  const wing = assets.get("wing")!;
  const options = {
    transition: hut.gameplay!.movementTransitions![0]!.id,
    appearance: "roof",
    node: "building-999",
    id: "roof-control",
    key: "hall-roof",
    rebase: (point: [number, number, number]) => point,
  };
  assert.throws(() => authorLinkedAppearance(hut, wing, options), /already has a control/);
  wing.gameplay!.movementTransitions = [];
  assert.throws(
    () => authorLinkedAppearance(hut, wing, { ...options, key: "other" }),
    /different shared control/,
  );
  assert.throws(
    () => authorLinkedAppearance(hut, wing, { ...options, node: "missing" }),
    /local identity/,
  );
  assert.throws(
    () => authorLinkedAppearance(hut, wing, { ...options, rebase: () => [NaN, 0, 0] }),
    /frame conversion/,
  );
});

test("shared appearance controls rebase contours and finite receiving anchors", () => {
  const { hut, assets } = joinedTransitionCompilerFixture();
  const wing = assets.get("wing")!;
  wing.gameplay!.movementTransitions = [];
  const control = hut.gameplay!.movementTransitions![0]!;
  control.waypointReceiverSegment = [
    [10, 20, -10],
    [10, 40, 10],
  ];
  const rebase = ([x, y, z]: [number, number, number]): [number, number, number] => [
    -y + 100,
    x + 50,
    z + 3,
  ];
  const result = authorLinkedAppearance(hut, wing, {
    transition: control.id,
    appearance: "roof",
    node: "building-999",
    id: "roof-control",
    key: "hall-roof",
    rebase,
  }).follower.movementTransitions![0]!;
  assert.deepEqual(result.waypoint, rebase(control.waypoint));
  assert.deepEqual(result.join!.point, rebase(control.join!.point));
  assert.deepEqual(result.waypointReceiverSegment, [
    [80, 60, -7],
    [60, 60, 13],
  ]);
  for (const key of ["applyPolygon", "noApplyPolygon"] as const)
    assert.deepEqual(
      result[key],
      control[key].map(([x, y]) => [-y + 100, x + 50]),
    );
});
