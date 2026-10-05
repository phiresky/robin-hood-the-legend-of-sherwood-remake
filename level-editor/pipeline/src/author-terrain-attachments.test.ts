import test from "node:test";
import assert from "node:assert/strict";
import {
  interiorAssetCompilerFixture,
  maskAssetCompilerFixture,
  anchoredReceiverCompilerFixture,
  assetCompilerFixture,
  terrainTransitionCompilerFixture,
} from "../../shared/test-fixtures/asset-gameplay.ts";
import {
  authorTerrainAttachments,
  type TerrainAttachmentRule,
} from "./author-terrain-attachments.ts";
import { compileAssetGameplay } from "../../shared/src/compile-asset-gameplay.ts";

test("control attachment recipes follow receiving terrain and reject stacked floors", () => {
  const { document, assets, hut } = terrainTransitionCompilerFixture();
  const control = hut.gameplay!.movementTransitions![0]!;
  delete control.waypointReceiverSegment;
  const rule: TerrainAttachmentRule = {
    kind: "control",
    id: control.id,
    node: control.node,
    anchor: control.waypoint,
    below: 10,
    above: 10,
  };
  const before = structuredClone(hut);
  const gameplay = authorTerrainAttachments(hut, [rule]);
  assert.deepEqual(hut, before);
  hut.gameplay = gameplay;
  assert.ok(
    compileAssetGameplay(document, assets, [0, 0, 2000, 2000]).movement_transitions!.length,
  );
  assert.deepEqual(authorTerrainAttachments(hut, [rule]), gameplay);
  assets.get("marker")!.gameplay!.surfaces[1]!.height = 7;
  assert.throws(
    () => compileAssetGameplay(document, assets, [0, 0, 2000, 2000]),
    /exactly one surface/,
  );
  hut.gameplay!.movementTransitions![0]!.waypointAnchor = control.waypoint;
  assert.throws(
    () => authorTerrainAttachments(hut, [rule]),
    /conflicts|invalid movement transition/,
  );
});

test("passage recipes author each endpoint independently and retain all door rules", () => {
  const { hut } = assetCompilerFixture();
  const door = hut.gameplay!.doors[0]!;
  const rules: TerrainAttachmentRule[] = (["outside", "inside"] as const).map((end) => ({
    kind: end === "outside" ? "passage-outside" : "passage-inside",
    id: door.id,
    node: door.node,
    anchor: door[end],
    below: 8,
    above: 8,
  }));
  const gameplay = authorTerrainAttachments(hut, rules);
  const { outsideReceiverSegment, insideReceiverSegment, ...rest } = gameplay.doors[0]!;
  assert.deepEqual(rest, door);
  assert.deepEqual(outsideReceiverSegment, [
    [80, 50, -8],
    [80, 50, 8],
  ]);
  assert.deepEqual(insideReceiverSegment, [
    [120, 50, -8],
    [120, 50, 8],
  ]);
  assert.equal(door.insideReceiverSegment, undefined);
  door.insideAnchor = door.inside;
  assert.throws(() => authorTerrainAttachments(hut, rules), /conflicts/);
});

test("changing attachment reach requires matching reviewed bounds and remains repeatable", () => {
  const { hut } = anchoredReceiverCompilerFixture();
  const receiver = hut.gameplay!.projectionReceivers![0]!;
  const original: TerrainAttachmentRule = {
    kind: "projection-receiver",
    id: receiver.id,
    node: receiver.node,
    anchor: receiver.anchor,
    below: 8,
    above: 8,
  };
  const rule: TerrainAttachmentRule = {
    ...original,
    below: 50,
    above: 50,
    replaceBounds: { below: 8, above: 8 },
  };
  assert.throws(() => authorTerrainAttachments(hut, [rule]), /no longer has reviewed bounds/);
  hut.gameplay = authorTerrainAttachments(hut, [original]);
  const before = structuredClone(hut);
  const updated = authorTerrainAttachments(hut, [rule]);
  assert.deepEqual(hut, before);
  assert.deepEqual(updated.projectionReceivers![0]!.receiverSegment, [
    [50, 50, -50],
    [50, 50, 50],
  ]);
  assert.deepEqual(authorTerrainAttachments({ ...hut, gameplay: updated }, [rule]), updated);
  assert.throws(
    () => authorTerrainAttachments(hut, [{ ...rule, replaceBounds: { below: 9, above: 8 } }]),
    /different bounds/,
  );
  assert.throws(
    () => authorTerrainAttachments(hut, [{ ...rule, replaceBounds: { below: -1, above: 8 } }]),
    /Invalid/,
  );
});

test("physical receiver recipes retain volumes and pin their owning frame", () => {
  const { hut } = anchoredReceiverCompilerFixture();
  const receiver = hut.gameplay!.projectionReceivers![0]!;
  const rule: TerrainAttachmentRule = {
    kind: "projection-receiver",
    id: receiver.id,
    node: receiver.node,
    anchor: receiver.anchor,
    below: 8,
    above: 8,
  };
  const result = authorTerrainAttachments(hut, [rule]);
  assert.deepEqual(result.projectionReceivers![0], {
    ...receiver,
    receiverSegment: [
      [50, 50, -8],
      [50, 50, 8],
    ],
  });
  assert.equal(hut.gameplay!.projectionReceivers![0]!.receiverSegment, undefined);
  assert.deepEqual(result.surfaces, hut.gameplay!.surfaces);
});

test("terrain attachment recipes are explicit, repeatable and do not mutate asset definitions", () => {
  const { hut } = interiorAssetCompilerFixture();
  const original = structuredClone(hut);
  const door = hut.gameplay!.interiors![0]!.doors[0]!;
  const rule: TerrainAttachmentRule = {
    kind: "interior-door",
    id: door.id,
    node: door.node,
    anchor: door.outside,
    below: 8,
    above: 8,
  };
  const gameplay = authorTerrainAttachments(hut, [rule]);
  assert.deepEqual(hut, original);
  assert.deepEqual(gameplay.interiors![0]!.doors[0]!.outsideReceiverSegment, [
    [20, 80, -8],
    [20, 80, 8],
  ]);
  assert.equal(gameplay.interiors![0]!.doors[1]!.outsideReceiverSegment, undefined);
  assert.deepEqual(authorTerrainAttachments({ ...hut, gameplay }, [rule]), gameplay);
  assert.throws(() => authorTerrainAttachments(hut, [rule, rule]), /Duplicate/);
  assert.throws(
    () => authorTerrainAttachments(hut, [{ ...rule, id: "missing" }]),
    /one local feature/,
  );
  assert.throws(
    () => authorTerrainAttachments(hut, [{ ...rule, anchor: [21, 80, 0] }]),
    /anchor changed/,
  );
  assert.throws(
    () => authorTerrainAttachments({ ...hut, gameplay }, [{ ...rule, above: 9 }]),
    /different bounds/,
  );
  door.outsideAnchor = [...door.outside];
  assert.throws(
    () => authorTerrainAttachments(hut, [{ ...rule, node: "changed" }]),
    /owner changed/,
  );
  assert.throws(() => authorTerrainAttachments(hut, [rule]), /conflicts/);
});

test("mask attachments preserve boundary rules and reject invalid reach", () => {
  const { hut } = maskAssetCompilerFixture();
  const mask = hut.gameplay!.masks![0]!;
  const rule: TerrainAttachmentRule = {
    kind: "mask",
    id: mask.id,
    node: mask.node,
    anchor: mask.anchor,
    below: 2,
    above: 3,
  };
  const gameplay = authorTerrainAttachments(hut, [rule]);
  const { receiverSegment, ...rest } = gameplay.masks![0]!;
  assert.deepEqual(receiverSegment, [
    [45, 45, -2],
    [45, 45, 3],
  ]);
  assert.deepEqual(rest, mask);
  for (const below of [-1, Infinity, NaN])
    assert.throws(() => authorTerrainAttachments(hut, [{ ...rule, below }]), /Invalid/);
  assert.throws(() => authorTerrainAttachments(hut, [{ ...rule, below: 0, above: 0 }]), /Invalid/);
});
