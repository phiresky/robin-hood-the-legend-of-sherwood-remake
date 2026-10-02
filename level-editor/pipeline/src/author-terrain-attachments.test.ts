import test from "node:test";
import assert from "node:assert/strict";
import {
  interiorAssetCompilerFixture,
  maskAssetCompilerFixture,
} from "../../shared/test-fixtures/asset-gameplay.ts";
import {
  authorTerrainAttachments,
  type TerrainAttachmentRule,
} from "./author-terrain-attachments.ts";

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
