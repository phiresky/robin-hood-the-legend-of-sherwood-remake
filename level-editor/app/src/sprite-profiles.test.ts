import test from "node:test";
import assert from "node:assert/strict";
import { bonusSprite, projectSpritePixel, spriteShape, sanitizedProfileName } from "./sprite-profiles.ts";

test("prone, dead, unconscious and tied poses use ground depth; sleeping upright stays upright", () => {
  for (const action of [45, 47, 48, 106, 108, 109, 113, 115, 116, 219]) assert.equal(spriteShape("character", action), "prone-character");
  for (const action of [0, 3, 14, 162]) assert.equal(spriteShape("character", action), "upright-character");
  assert.equal(spriteShape("pickup", 190), "low-object");
  assert.equal(spriteShape("scenery", 0), "upright-scenery");
  assert.equal(spriteShape("pickup", 190, "BONUS_Shield"), "cylinder-object");
  assert.equal(spriteShape("pickup", 194, "bonus_shield"), "cylinder-object");
  assert.equal(spriteShape("pickup", 190, "BONUS_MoneyBag"), "low-object");
});

test("all sprite shapes preserve source projection; corpses stay within ten units of the ground", () => {
  const elevation = 35 * Math.PI / 180;
  const bounds = { left: -35, top: 24, width: 70, height: 45 };
  for (const shape of ["upright-character", "prone-character", "cylinder-object", "low-object", "upright-scenery"] as const) {
    for (let x = -35; x <= 35; x += 7) for (let up = -21; up <= 24; up += 5) {
      const p = projectSpritePixel(shape, x, up, bounds, elevation);
      assert.ok(Math.abs(p[1] * Math.cos(elevation) - p[2] * Math.sin(elevation) - up) < 1e-8);
      if (shape === "prone-character") assert.ok(p[1] >= 0 && p[1] <= 10);
    }
  }
});

test("bonus catalog includes all 19 types and preserves exact profile names", () => {
  assert.deepEqual(bonusSprite(0), ["BONUS_Arrows", "BONUS Fleches"]);
  assert.deepEqual(bonusSprite(13), ["RELIC_Spoon", "Cuillere"]);
  for (let i = 0; i < 19; i++) assert.equal(bonusSprite(i).length, 2);
  assert.throws(() => bonusSprite(19), /Unknown bonus/);
  assert.equal(sanitizedProfileName('..Bridge: A/B..'), 'Bridge_ A_B');
});
