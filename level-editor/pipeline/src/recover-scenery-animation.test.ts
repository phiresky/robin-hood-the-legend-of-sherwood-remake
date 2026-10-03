import test from "node:test";
import assert from "node:assert/strict";
import { recoverSceneryAnimation } from "./recover-scenery-animation.ts";
import { compileSceneryAnimation } from "../../shared/src/compile-scenery-animation.ts";
import type { ElementFx } from "../../shared/src/level.ts";
import type { Vec3 } from "../../shared/src/scene.ts";

const source: ElementFx = {
  sprite: {
    frame_profile_name: "flame",
    profile_name: "burning",
    position_x: 100,
    position_y: 200,
    elevation: 0,
  },
  blit_type: 0,
  active: true,
  force_display: true,
  display_polyline: [
    [95, 220],
    [120, 220],
  ],
};
const profile = { name: "burning", center_x: 4, center_y: 6 };
const owner = { id: "candle", node: "table", anchor: [104, 246, 40] as Vec3 };

test("reviewed scenery height keeps screen placement while moving with a rotated owner", () => {
  const localize = ([x, y, z]: Vec3): Vec3 => [y - z - 200, 100 - x + z - 10, z - 10];
  const world = (_node: string, [x, y, z]: Vec3): Vec3 => [100 - (y - z), 200 + x + z + 10, z + 10];
  const animation = recoverSceneryAnimation(source, profile, owner, localize);
  assert.deepEqual(animation.anchor, [6, 26, 30]);
  const recovered = compileSceneryAnimation(animation, world);
  assert.deepEqual(recovered, { ...source, sprite: { ...source.sprite, elevation: 40 } });
  const moved = compileSceneryAnimation(animation, (node, point) => {
    const [x, y, z] = world(node, point);
    return [x + 300, y + 520, z + 20];
  });
  assert.deepEqual(moved.sprite, {
    ...source.sprite,
    position_x: 400,
    position_y: 700,
    elevation: 60,
  });
  assert.deepEqual(moved.display_polyline, [
    [395, 720],
    [420, 720],
  ]);
  assert.equal("source" in animation, false);
});

test("recovery refuses mismatched profiles, screen positions and invalid local frames", () => {
  assert.throws(
    () => recoverSceneryAnimation(source, { ...profile, name: "other" }, owner, (p) => p),
    /reviewed anchor/,
  );
  assert.throws(
    () => recoverSceneryAnimation(source, profile, { ...owner, anchor: [104, 206, 40] }, (p) => p),
    /reviewed anchor/,
  );
  assert.throws(
    () => recoverSceneryAnimation(source, profile, owner, () => [NaN, 0, 0]),
    /owner transform/,
  );
});
