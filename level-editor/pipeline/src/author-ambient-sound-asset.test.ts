import test from "node:test";
import assert from "node:assert/strict";
import { NodeIO } from "@gltf-transform/core";
import { authorAmbientSoundAsset } from "./author-ambient-sound-asset.ts";
import { recoverAuthoredSounds } from "./recover-authored-sounds.ts";
import { assetCompilerFixture } from "../../shared/test-fixtures/asset-gameplay.ts";
import { compileAssetGameplay } from "../../shared/src/compile-asset-gameplay.ts";
import type { SoundSource } from "../../shared/src/level.ts";

const sound: SoundSource = {
  id: 54,
  active: true,
  source_kind: 2,
  delayed_params: [150, 500, 5],
  global: false,
  inner_distance: 80,
  outer_distance: 600,
  polyline: [
    [-14, 508],
    [101, 1614],
    [80, 2628],
  ],
  inner_volume: 100,
  outer_volume: 0,
  noise_covering_distance: 0,
  altitude: 1,
  ambience_filter: 255,
};
test("standalone ambient assets compile exactly, remain invisible, and move independently", async () => {
  const { descriptor, model, placement } = await authorAmbientSoundAsset(sound, {
    id: "west-ambient",
    name: "West ambient zone",
    map: "test",
    origin: [-14, 508, 0],
  });
  const gltf = await new NodeIO().readBinary(model);
  assert.equal(gltf.getRoot().listMeshes().length, 0);
  assert.equal(
    gltf
      .getRoot()
      .getDefaultScene()!
      .listChildren()[0]!
      .listChildren()[0]!
      .listChildren()[0]!
      .getName(),
    descriptor.parts[0]!.node,
  );
  assert.equal(descriptor.parts[0]!.gameplay_only, true);
  const { document, assets } = assetCompilerFixture();
  assets.set(descriptor.id, descriptor);
  document.objects.push(placement);
  const bounds: [number, number, number, number] = [0, 0, 3000, 3000];
  const original = compileAssetGameplay(document, assets, bounds);
  assert.deepEqual(original.sound_sources, [sound]);
  const recovered = recoverAuthoredSounds(document, assets, [sound]);
  assert.equal(recovered.length, 1);
  assert.deepEqual(recovered[0]!.sourceIndices, [0]);
  // Publication attaches review metadata and empty traversal collections.
  descriptor.gameplay!.lifts = [];
  descriptor.gameplay!.interiors = [];
  descriptor.gameplay!.draft = { issues: ["Review ambient extent"] };
  assert.deepEqual(recoverAuthoredSounds(document, assets, [sound]), recovered);
  recovered[0]!.sounds[0]!.spatial!.polyline[0]![0] = 999;
  assert.equal(descriptor.gameplay!.sounds![0]!.spatial!.polyline[0]![0], 0);
  assert.throws(
    () => recoverAuthoredSounds(document, assets, [sound, sound]),
    /exactly one unclaimed/,
  );
  placement.transform.dx += 100;
  assert.throws(() => recoverAuthoredSounds(document, assets, [sound]), /exactly one unclaimed/);
  const moved = compileAssetGameplay(document, assets, bounds);
  assert.deepEqual(moved.sound_sources, [
    { ...sound, polyline: sound.polyline!.map(([x, y]) => [x + 100, y]) },
  ]);
  assert.deepEqual(moved.motion_data, original.motion_data);
  assert.deepEqual(moved.sight_obstacles, original.sight_obstacles);
  assert.deepEqual(moved.doors, original.doors);
  document.objects.push({
    ...structuredClone(placement),
    id: "copy",
    transform: { ...placement.transform, dx: 200 },
  });
  assert.equal(compileAssetGameplay(document, assets, bounds).sound_sources!.length, 2);
  assert.throws(
    () => recoverAuthoredSounds(document, assets, [sound]),
    /exactly one placed instance/,
  );
  document.objects = document.objects.filter((p) => !p.node.startsWith(`asset:${descriptor.id}:`));
  assert.throws(
    () => recoverAuthoredSounds(document, assets, [sound]),
    /exactly one placed instance/,
  );
});

test("ambient authoring rejects global, incomplete and invalid definitions", async () => {
  const options = {
    id: "ambient",
    name: "Ambient",
    map: "test",
    origin: [0, 0, 0] as [number, number, number],
  };
  await assert.rejects(
    authorAmbientSoundAsset({ ...sound, global: true }, options),
    /terrain metadata/,
  );
  await assert.rejects(
    authorAmbientSoundAsset({ ...sound, polyline: null }, options),
    /incomplete/,
  );
  await assert.rejects(
    authorAmbientSoundAsset(sound, { ...options, id: "../escape" }),
    /stable ID/,
  );
  await assert.rejects(
    authorAmbientSoundAsset(sound, { ...options, origin: [0, NaN, 0] }),
    /finite/,
  );
});
