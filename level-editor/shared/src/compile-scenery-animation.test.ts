import test from "node:test";
import assert from "node:assert/strict";
import { compileSceneryAnimation } from "./compile-scenery-animation.ts";
import { compileAssetGameplay } from "./compile-asset-gameplay.ts";
import { validateAssetGameplay, type AssetSceneryAnimation } from "./asset-gameplay.ts";
import { assetCompilerFixture } from "../test-fixtures/asset-gameplay.ts";

const animation: AssetSceneryAnimation = {
  id: "flame",
  node: "building-999",
  anchor: [10, 30, 20],
  file: "torch.rhs",
  profile: "burning",
  center: [4, 6],
  active: true,
  forceDisplay: false,
  shadow: true,
  displayPolyline: [
    [0, 20, 20],
    [20, 20, 20],
  ],
};

test("scenery rotates its local anchor and mask while retaining its billboard center", () => {
  for (const file of ["torch", "torch.rhs", "torch.RHS"])
    assert.equal(
      compileSceneryAnimation({ ...animation, file }, (_node, point) => point).sprite
        .frame_profile_name,
      "torch",
    );
  const result = compileSceneryAnimation(animation, (_node, [x, y, z]) => [
    100 - (y - z),
    200 + x + z + 10,
    z + 10,
  ]);
  assert.deepEqual(result, {
    sprite: {
      frame_profile_name: "torch",
      profile_name: "burning",
      position_x: 86,
      position_y: 204,
      elevation: 30,
    },
    blit_type: 1,
    active: true,
    force_display: false,
    display_polyline: [
      [100, 200],
      [100, 220],
    ],
  });
  assert.throws(() => compileSceneryAnimation(animation, () => [0, 0, -1]), /runtime range/);
  assert.throws(() => compileSceneryAnimation(animation, () => [40000, 0, 0]), /runtime range/);
});

test("asset scenery reaches compiled map data and unavailable placement warns in best effort", () => {
  const { document, assets, hut } = assetCompilerFixture();
  hut.gameplay!.animations = [structuredClone(animation)];
  const compile = (bestEffort = false) =>
    compileAssetGameplay(document, assets, [0, 0, 2000, 2000], { bestEffort });
  const original = compile();
  assert.equal(original.animations?.length, 1);
  assert.equal(original.animations![0]!.sprite.frame_profile_name, "torch");
  const copy = structuredClone(document.objects[0]!);
  copy.id = "hut-b-body";
  copy.group = "hut-b";
  copy.transform.dx += 500;
  document.objects.push(copy);
  document.groups.push({ ...structuredClone(document.groups[0]!), id: "hut-b" });
  const duplicated = compile();
  assert.equal(duplicated.animations?.length, 2);
  const [first, second] = duplicated.animations!;
  assert.equal(second!.sprite.position_x - first!.sprite.position_x, 500);
  assert.equal(second!.sprite.position_y, first!.sprite.position_y);
  assert.equal(second!.display_polyline[0]![0] - first!.display_polyline[0]![0], 500);
  document.objects.pop();
  document.groups.pop();
  hut.gameplay!.animations[0]!.anchor[2] = -1000;
  assert.throws(() => compile(), /runtime range/);
  const partial = compile(true);
  assert.equal(partial.animations, undefined);
  assert.deepEqual(partial.sight_obstacles, original.sight_obstacles);
  assert.ok(
    partial.warnings?.some(
      (warning) => warning.includes("Animation") && warning.includes("omitted"),
    ),
  );
  hut.gameplay!.animations[0]!.profile = "";
  assert.throws(() => validateAssetGameplay(hut.gameplay, hut), /invalid scenery animation/);
  hut.gameplay!.animations[0]!.profile = "burning";
  hut.gameplay!.animations[0]!.file = ".rhs";
  assert.throws(() => validateAssetGameplay(hut.gameplay, hut), /invalid scenery animation/);
});

test("reversed placement preserves left-to-right scenery ordering boundaries", () => {
  const bent = {
    ...animation,
    displayPolyline: [
      [0, 20, 20],
      [10, 35, 20],
      [20, 40, 20],
    ] as [number, number, number][],
  };
  const compiled = compileSceneryAnimation(bent, (_node, [x, y, z]) => [
    100 - x,
    200 - (y - z) + z,
    z,
  ]);
  assert.deepEqual(compiled.display_polyline, [
    [80, 180],
    [90, 185],
    [100, 200],
  ]);
  assert.deepEqual(bent.displayPolyline, [
    [0, 20, 20],
    [10, 35, 20],
    [20, 40, 20],
  ]);
});
