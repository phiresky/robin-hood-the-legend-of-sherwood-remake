import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { createHash } from "node:crypto";
import { prepareActorPixels, type ActorMaskQuery } from "./native-actor-pixels.ts";
const root = resolve(import.meta.dirname, "../../..");
const fixture = JSON.parse(
  readFileSync(
    resolve(
      root,
      "level-editor/work/croisement02-refinement/restart20-hidden-outline-v1/fixture.json",
    ),
    "utf8",
  ),
);
const level = JSON.parse(readFileSync(resolve(root, fixture.level.path), "utf8"));
const hash = (data: Uint8Array) => createHash("sha256").update(data).digest("hex");
const query: ActorMaskQuery = {
  ...fixture.actor,
  screenOrigin: fixture.screenOrigin,
  drawHidden: true,
  outlineColor: 0xf800,
  depth: 16,
  shadowKey: 31,
  shadowStrength: 40,
};
test("promoted actor decoder matches independent packed Archer/mask oracle", () => {
  const source = { ...fixture.source, data: new Uint8Array(fixture.source.data) },
    original = source.data.slice();
  const got = prepareActorPixels(source, query, [
    { id: "mask14", mask: level.masks[fixture.maskIndex], active: true },
  ]);
  const packed = Buffer.alloc(source.width * source.height * 2);
  for (let i = 0; i < source.width * source.height; i++) {
    const d = got.pixels.data;
    packed.writeUInt16LE(
      d[i * 4 + 3] === 0
        ? 0x7c0
        : ((d[i * 4]! >> 3) << 11) | ((d[i * 4 + 1]! >> 2) << 5) | (d[i * 4 + 2]! >> 3),
      i * 2,
    );
  }
  assert.equal(hash(packed), fixture.expectedPackedSha256);
  assert.equal(got.applied[0]!.outlined, fixture.expectedOutlined);
  assert.deepEqual(source.data, original);
});
test("transport key uses actual ambient key and cannot reinterpret a blue outline as shadow", () => {
  const got = prepareActorPixels(
    { width: 2, height: 1, data: new Uint8Array([0, 0, 255, 255, 0, 248, 0, 255]) },
    { ...query, screenOrigin: [0, 0], mapPosition: [0, 0], shadowKey: 0x7bef, outlineColor: 31 },
    [],
  );
  assert.deepEqual(got.shadow.rgb, [123, 125, 123]);
  assert.deepEqual([...got.pixels.data], [123, 125, 123, 255, 0, 0, 0, 0]);
  assert.equal(got.shadowPixels, 1);
});
test("repeated snapshots preserve source bytes and invalid or duplicate current inputs fail", () => {
  const source = { ...fixture.source, data: new Uint8Array(fixture.source.data) };
  const masks = [{ id: "mask14", mask: level.masks[fixture.maskIndex], active: true }];
  assert.deepEqual(
    prepareActorPixels(source, query, masks),
    prepareActorPixels(source, query, masks),
  );
  assert.throws(() => prepareActorPixels(source, query, [...masks, ...masks]), /Duplicate/);
  assert.throws(
    () => prepareActorPixels(source, { ...query, mapPosition: [NaN, 0] }, []),
    /policy/,
  );
  source.data[3] = 128;
  assert.throws(() => prepareActorPixels(source, query, []), /Filtered/);
});
