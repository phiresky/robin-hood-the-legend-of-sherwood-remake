import test from "node:test";
import assert from "node:assert/strict";
import { translateGameplayFrames } from "./translate-gameplay-frames.ts";
import { compileAssetGameplay } from "../../shared/src/compile-asset-gameplay.ts";
import { validateAssetGameplay } from "../../shared/src/asset-gameplay.ts";
import type { Vec3 } from "../../shared/src/scene.ts";
import { gameTransformMatrix, groupCentroid, groupParts } from "../../shared/src/level3d.ts";
import {
  assetCompilerFixture,
  maskAssetCompilerFixture,
  anchoredReceiverCompilerFixture,
  slopedAssetCompilerFixture,
  changingLiftCompilerFixture,
  liftLightCompilerFixture,
  interiorAssetCompilerFixture,
  soundAssetCompilerFixture,
  sightTransitionCompilerFixture,
  jumpAssetCompilerFixture,
  doorTransitionCompilerFixture,
  projectionMaterialCompilerFixture,
  projectionVolumeCompilerFixture,
  rotatedSceneryCompilerFixture,
  clearanceAssetCompilerFixture,
} from "../../shared/test-fixtures/asset-gameplay.ts";

function equivalent(actual: unknown, expected: unknown, path = "") {
  if (typeof actual === "number" && typeof expected === "number") {
    assert.ok(Math.abs(actual - expected) < 1e-6, `${path}: ${actual} != ${expected}`);
  } else if (actual && expected && typeof actual === "object" && typeof expected === "object") {
    assert.deepEqual(Object.keys(actual), Object.keys(expected), path);
    for (const key of Object.keys(actual))
      equivalent(Reflect.get(actual, key), Reflect.get(expected, key), `${path}/${key}`);
  } else assert.deepEqual(actual, expected, path);
}

test("translated clearance frames retain per-vertex navigation heights", () => {
  const { assets } = clearanceAssetCompilerFixture();
  const asset = [...assets.values()].find((asset) => asset.gameplay?.movementClearances?.length)!;
  const gameplay = structuredClone(asset.gameplay!);
  const clearance = gameplay.movementClearances![0]!;
  clearance.navigationHeight = clearance.polygon.map(([x, y]) => x * 0.1 + y * 0.2);
  const original = structuredClone(gameplay);
  const shifted = translateGameplayFrames(
    gameplay,
    new Map([[clearance.node, [10, 20, 30] as Vec3]]),
  );
  assert.deepEqual(
    shifted.movementClearances![0]!.navigationHeight,
    clearance.navigationHeight.map((z) => z + 30),
  );
  equivalent(
    translateGameplayFrames(shifted, new Map([[clearance.node, [-10, -20, -30] as Vec3]])),
    original,
  );
  assert.deepEqual(gameplay, original);
});

for (const fixture of [
  assetCompilerFixture,
  maskAssetCompilerFixture,
  anchoredReceiverCompilerFixture,
  slopedAssetCompilerFixture,
  changingLiftCompilerFixture,
  liftLightCompilerFixture,
  interiorAssetCompilerFixture,
  soundAssetCompilerFixture,
  sightTransitionCompilerFixture,
  jumpAssetCompilerFixture,
  doorTransitionCompilerFixture,
  projectionMaterialCompilerFixture,
  projectionVolumeCompilerFixture,
  rotatedSceneryCompilerFixture,
  clearanceAssetCompilerFixture,
])
  test(`translated authoring frames preserve compiled geometry: ${fixture.name}`, () => {
    const { document, assets } = fixture();
    const before = compileAssetGameplay(document, assets, [0, 0, 2000, 2000]);
    const groupMatrices = document.groups.map((group) =>
      gameTransformMatrix(
        document.camera,
        group.transform,
        groupCentroid(groupParts(document, group.id)),
      ),
    );
    const offset: Vec3 = [128, -64, 32];
    for (const asset of assets.values()) {
      if (!asset.gameplay) continue;
      const original = structuredClone(asset.gameplay);
      asset.gameplay = translateGameplayFrames(
        asset.gameplay,
        new Map(asset.parts.map((p) => [p.node, offset])),
      );
      equivalent(
        translateGameplayFrames(
          asset.gameplay,
          new Map(asset.parts.map((p) => [p.node, [-128, 64, -32]])),
        ),
        original,
      );
      for (const part of asset.parts) {
        if (part.obstacle_local_game)
          part.obstacle_local_game = {
            ...part.obstacle_local_game,
            points: part.obstacle_local_game.points.map((p) => ({
              ...p,
              x: p.x + 128,
              y: p.y - 64,
              z_bottom: p.z_bottom + 32,
              z_top: p.z_top + 32,
            })),
          };
        for (const object of document.objects.filter(
          (o) => o.node === `asset:${asset.id}:${part.node}`,
        )) {
          assert.equal(object.transform.rot_deg, 0);
          object.obstacle = part.obstacle_local_game;
          object.transform.dx -= 128;
          object.transform.dy += 64;
          object.transform.dz -= 32;
        }
      }
      validateAssetGameplay(asset.gameplay, asset);
    }
    // Part bounds define the rotation pivot. Keep the world transform unchanged
    // when authoring changes those bounds to a different local frame.
    for (const [index, group] of document.groups.entries()) {
      const after = gameTransformMatrix(
        document.camera,
        group.transform,
        groupCentroid(groupParts(document, group.id)),
      );
      group.transform.dx += groupMatrices[index]![12]! - after[12]!;
      group.transform.dy -=
        (groupMatrices[index]![13]! - after[13]!) *
        Math.sin((document.camera.elevation_deg * Math.PI) / 180);
    }
    equivalent(compileAssetGameplay(document, assets, [0, 0, 2000, 2000]), before);
  });

test("frame migration rejects missing transforms and stale spline calibration", () => {
  const gameplay = assetCompilerFixture().hut.gameplay!;
  assert.throws(() => translateGameplayFrames(gameplay, new Map()), /Missing finite frame offset/);
  gameplay.spline = { bounds: { min: [0, 0, 0], max: [1, 1, 1] }, frames: {} };
  assert.throws(() => translateGameplayFrames(gameplay, new Map()), /Recalibrate spline/);
  delete gameplay.spline;
  gameplay.placementGroundHeight = 0;
  assert.throws(
    () =>
      translateGameplayFrames(
        gameplay,
        new Map([
          ["a", [0, 0, 1]],
          ["b", [0, 0, 2]],
        ]),
      ),
    /shared vertical/,
  );
});
