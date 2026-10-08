import test from "node:test";
import assert from "node:assert/strict";
import {
  recoverSoundSource,
  containsSoundPolyline,
  uniqueSoundOwner,
  declaredSoundOwners,
} from "./recover-sound-source.ts";
import { compileAssetGameplay } from "../../shared/src/compile-asset-gameplay.ts";
import { soundAssetCompilerFixture } from "../../shared/test-fixtures/asset-gameplay.ts";
import { wallDisconnectedSoundFixture } from "../../shared/test-fixtures/wall-spline.ts";
import { compileSoundSource } from "../../shared/src/compile-sound-source.ts";

test("sound extraction retains disconnected fragments through local frame translation", () => {
  const { document, assets, bounds } = wallDisconnectedSoundFixture();
  const raw = compileAssetGameplay(document, assets, bounds).sound_sources![0]!;
  const local = recoverSoundSource(raw, "wind", "body", ([x, y, z]) => [x - 100, y - 200, z]);
  assert.deepEqual(local.spatial!.polylineBreaks, [2]);
  assert.deepEqual(
    compileSoundSource(local, (_, [x, y, z]) => [x + 100, y + 200, z]),
    raw,
  );
});

test("reviewed sound ownership chooses one frame despite overlapping asset footprints", () => {
  const { document, assets } = soundAssetCompilerFixture();
  const sources = compileAssetGameplay(document, assets, [0, 0, 2000, 2000]).sound_sources!;
  const frame = { asset: "hut", node: "building-999" };
  const entry = {
    source: 0,
    owner: frame.asset,
    node: frame.node,
    reason: "Attach the emitter to the building",
    sound: sources[0]!,
  };
  const resolve = (asset: string, node: string) =>
    asset === frame.asset && node === frame.node ? [frame] : [];
  assert.equal(declaredSoundOwners([entry], sources, resolve).get(0), frame);
  assert.throws(
    () => declaredSoundOwners([entry, entry], sources, resolve),
    /unclaimed local source/,
  );
  assert.throws(
    () => declaredSoundOwners([entry], sources, resolve, new Set([0])),
    /unclaimed local source/,
  );
  assert.throws(
    () => declaredSoundOwners([{ ...entry, reason: " " }], sources, resolve),
    /reviewed asset frame/,
  );
  assert.throws(
    () => declaredSoundOwners([{ ...entry, source: 1, sound: sources[1]! }], sources, resolve),
    /unclaimed local source/,
  );
  assert.throws(
    () => declaredSoundOwners([{ ...entry, source: 0.5 }], sources, resolve),
    /unclaimed local source/,
  );
  assert.throws(
    () => declaredSoundOwners([{ ...entry, owner: "missing" }], sources, resolve),
    /one pinned frame/,
  );
  assert.throws(
    () => declaredSoundOwners([entry], sources, () => [frame, frame]),
    /one pinned frame/,
  );
  assert.throws(
    () =>
      declaredSoundOwners(
        [{ ...entry, sound: { ...entry.sound, inner_volume: 1 } }],
        sources,
        resolve,
      ),
    /source changed/,
  );
});

test("overlapping parts share sound ownership only within the same asset", () => {
  const first = { asset: "tower", node: "building-001", frame: [10, 20] };
  const second = { asset: "tower", node: "building-002", frame: [30, 40] };
  assert.equal(uniqueSoundOwner([second, first]), first);
  assert.equal(uniqueSoundOwner([first, second]), first);
  assert.equal(uniqueSoundOwner([first]), first);
  assert.equal(uniqueSoundOwner([]), undefined);
  assert.equal(uniqueSoundOwner([first, second, { ...first, asset: "terrain" }]), undefined);
});

test("sound ownership checks segment interiors, not just vertices inside a concave part", () => {
  const boundary: [number, number][] = [
    [0, 0],
    [10, 0],
    [10, 10],
    [7, 10],
    [7, 3],
    [3, 3],
    [3, 10],
    [0, 10],
  ];
  assert.equal(
    containsSoundPolyline(
      [
        [1, 8],
        [9, 8],
      ],
      boundary,
    ),
    false,
  );
  assert.equal(
    containsSoundPolyline(
      [
        [1, 8],
        [1, 1],
        [9, 1],
        [9, 8],
      ],
      boundary,
    ),
    true,
  );
  assert.equal(
    containsSoundPolyline(
      [
        [3, 4],
        [3, 9],
      ],
      boundary,
    ),
    true,
  );
  assert.equal(containsSoundPolyline([[12, 0]], boundary), false);
});

test("one-time sound recovery reconstructs all source semantics after local placement", () => {
  const { document, assets, hut } = soundAssetCompilerFixture();
  const expected = compileAssetGameplay(document, assets, [0, 0, 2000, 2000]).sound_sources!;
  hut.gameplay!.sounds = expected.map((raw, i) =>
    recoverSoundSource(raw, `sound-${i}`, "building-999", ([x, y, z]) => [x - 300, y - 300, z]),
  );
  assert.deepEqual(
    compileAssetGameplay(document, assets, [0, 0, 2000, 2000]).sound_sources,
    expected,
  );
  const bad = structuredClone(expected[0]!);
  bad.inner_volume = null;
  assert.throws(
    () => recoverSoundSource(bad, "bad", "building-999", (p) => p),
    /incomplete spatial data/,
  );
});
