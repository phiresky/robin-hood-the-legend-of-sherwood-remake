import test from "node:test";
import assert from "node:assert/strict";
import { NodeIO } from "@gltf-transform/core";
import { authorSceneryAnimationAsset } from "./author-scenery-animation-asset.ts";
import { assetCompilerFixture } from "../../shared/test-fixtures/asset-gameplay.ts";
import { compileAssetGameplay } from "../../shared/src/compile-asset-gameplay.ts";
import type { AssetSceneryAnimation } from "../../shared/src/asset-gameplay.ts";
import type { Vec3 } from "../../shared/src/scene.ts";
import { parseStoredMap, serializeStoredMap } from "../../shared/src/stored-level.ts";

const animation: Omit<AssetSceneryAnimation, "node"> = {
  id: "flame",
  anchor: [4, 6, 0],
  file: "flame",
  profile: "burning",
  center: [4, 6],
  active: true,
  forceDisplay: false,
  shadow: false,
  displayPolyline: [
    [0, 0, 0],
    [20, 0, 0],
  ],
};
const options = {
  id: "campfire",
  name: "Campfire",
  map: "authored",
  origin: [100, 200, 0] as Vec3,
};

test("standalone effects export independently after copy, rotation and elevation", async () => {
  const { descriptor, model, placement } = await authorSceneryAnimationAsset([animation], options);
  const gltf = await new NodeIO().readBinary(model);
  assert.equal(
    gltf.getRoot().listMeshes().length,
    0,
    "sprite artwork must not enter the static bake",
  );
  assert.ok(
    gltf
      .getRoot()
      .listNodes()
      .some((node) => node.getName() === descriptor.parts[0]!.node),
  );
  const { document, assets } = assetCompilerFixture();
  const bounds: [number, number, number, number] = [0, 0, 2000, 2000];
  const before = compileAssetGameplay(document, assets, bounds);
  assets.set(descriptor.id, descriptor);
  document.assetSources!.push({
    id: descriptor.id,
    descriptor: `3d-assets/${descriptor.id}/asset.json`,
    model: `3d-assets/${descriptor.id}/model.glb`,
    descriptor_sha256: "a".repeat(64),
    model_sha256: "b".repeat(64),
    model_scene: "default",
    resources: [],
  });
  document.objects.push(placement);
  const initial = compileAssetGameplay(document, assets, bounds);
  assert.deepEqual(initial.animations![0]!.sprite, {
    frame_profile_name: "flame",
    profile_name: "burning",
    position_x: 100,
    position_y: 200,
    elevation: 0,
  });
  assert.deepEqual(initial.animations![0]!.display_polyline, [
    [100, 200],
    [120, 200],
  ]);
  assert.deepEqual(initial.motion_data, before.motion_data);
  assert.deepEqual(initial.sight_obstacles, before.sight_obstacles);
  const copy = structuredClone(placement);
  copy.id = "other-fire";
  copy.transform = { dx: 400, dy: 500, dz: 20, rot_deg: 180 };
  document.objects.push(copy);
  const moved = compileAssetGameplay(document, assets, bounds);
  assert.deepEqual(moved.animations![0], initial.animations![0]);
  assert.deepEqual(moved.animations![1]!.sprite, {
    frame_profile_name: "flame",
    profile_name: "burning",
    position_x: 392,
    position_y: 468,
    elevation: 20,
  });
  assert.deepEqual(moved.animations![1]!.display_polyline, [
    [380, 480],
    [400, 480],
  ]);
  const reopened = parseStoredMap(serializeStoredMap(document, assets), assets);
  assert.deepEqual(compileAssetGameplay(reopened, assets, bounds).animations, moved.animations);
  document.objects = document.objects.filter((part) => part.id !== placement.id);
  assert.deepEqual(compileAssetGameplay(document, assets, bounds).animations, [
    moved.animations![1],
  ]);
});

test("authoring isolates local definitions and pinned resources from caller mutations", async () => {
  const input = structuredClone(animation);
  input.resourceDirectory = "effects/fire.rhs.d";
  const resources = [{ path: "effects/fire.rhs.d/manifest.json", sha256: "a".repeat(64) }];
  const { descriptor } = await authorSceneryAnimationAsset([input], { ...options, resources });
  input.anchor[0] = 900;
  input.displayPolyline[0]![0] = 800;
  resources[0]!.sha256 = "b".repeat(64);
  assert.equal(descriptor.gameplay!.animations![0]!.anchor[0], 4);
  assert.equal(descriptor.gameplay!.animations![0]!.displayPolyline[0]![0], 0);
  assert.equal(descriptor.gameplay!.animations![0]!.resourceDirectory, "effects/fire.rhs.d");
  assert.equal(descriptor.resources![0]!.sha256, "a".repeat(64));
});

test("standalone scenery authoring rejects empty, invalid and duplicate effects", async () => {
  await assert.rejects(authorSceneryAnimationAsset([], options), /at least one/);
  await assert.rejects(
    authorSceneryAnimationAsset([animation], { ...options, id: "../fire" }),
    /stable ID/,
  );
  await assert.rejects(
    authorSceneryAnimationAsset([animation], { ...options, origin: [0, NaN, 0] }),
    /finite/,
  );
  await assert.rejects(
    authorSceneryAnimationAsset([{ ...animation, profile: "" }], options),
    /invalid scenery/,
  );
  await assert.rejects(authorSceneryAnimationAsset([animation, animation], options));
});
