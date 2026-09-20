import test from "node:test";
import assert from "node:assert/strict";
import type { SightObstacle } from "@rle/shared";
import { cylinderPixel, placementHeight, viewedDirection } from "./entity-projection.ts";

test("all 16 headings rotate relative to the viewer and wrap across north", () => {
  for (let d = 0; d < 16; d++) {
    assert.equal(viewedDirection(d, 0), d);
    assert.equal(viewedDirection(d, Math.PI / 2), (d + 4) % 16);
    assert.equal(viewedDirection(d, -Math.PI / 2), (d + 12) % 16);
    assert.equal(viewedDirection(d, Math.PI * 2), d);
  }
  assert.equal(viewedDirection(0, -0.01), 0);
});

test("elevated and sloping supports recover world height from projected placements", () => {
  const obstacle = (slope: number) => ({ points: [
    { x: 0, y: 0, z_top: 100 }, { x: 100, y: 0, z_top: 100 },
    { x: 100, y: 100, z_top: 100 + slope * 100 },
  ] }) as SightObstacle;
  assert.equal(placementHeight(20, 30, obstacle(0)), 100);
  // z = 100 + 0.5*y and mapY = y-z => y=260, z=230.
  assert.equal(placementHeight(20, 30, obstacle(0.5)), 230);
  assert.throws(() => placementHeight(20, 30, obstacle(1)), /parallel/);
});

test("cylinder reprojection preserves every source pixel at the game elevation", () => {
  const elevation = 35 * Math.PI / 180;
  for (let x = -30; x <= 30; x += 3) for (let y = -5; y <= 70; y += 3) {
    const p = cylinderPixel(x, y, 12, 70, elevation);
    assert.equal(p[0], x);
    assert.ok(Math.abs(p[1] * Math.cos(elevation) - p[2] * Math.sin(elevation) - y) < 1e-10);
    assert.ok(p.every(Number.isFinite));
  }
  const cap = cylinderPixel(0, 60, 12, 70, elevation);
  assert.ok(Math.abs(cap[1] - 70) < 1e-10);
  const body = cylinderPixel(0, 30, 12, 70, elevation);
  assert.equal(body[2], 12);
  // Looking down on the cap reveals depth rather than flattening to a line.
  assert.notEqual(cap[2], body[2]);
});

test("legacy PNG transparency and shadow keys are removed before texture filtering", async () => {
  const { decodeSpritePixels } = await import("./entity-projection.ts");
  const rgba = new Uint8ClampedArray([0,251,0,255, 0,0,255,255, 70,100,40,255, 0,255,0,255]);
  const authored = rgba.slice();
  decodeSpritePixels(authored, false);
  assert.deepEqual(authored, rgba);
  decodeSpritePixels(rgba, true);
  assert.deepEqual([...rgba], [0,0,0,0, 0,0,0,0, 70,100,40,255, 0,255,0,255]);
});

test("authored shadow masks preserve coverage and use mission ambiance blending", async () => {
  const { spriteShadowPixels, spriteShadowStyle } = await import("./entity-projection.ts");
  const pixels = new Uint8ClampedArray([0,0,255,255, 0,251,0,255, 60,60,70,255]);
  assert.deepEqual([...spriteShadowPixels(pixels, true)!], [255,255,255,255, 0,0,0,0, 0,0,0,0]);
  assert.equal(spriteShadowPixels(pixels, false), null);
  assert.deepEqual(spriteShadowStyle("Day"), { color: 0, opacity: 0.4 });
  assert.deepEqual(spriteShadowStyle("Night"), { color: 0, opacity: 0.4 });
  assert.deepEqual(spriteShadowStyle("Fog"), { color: 0, opacity: 0.1 });
});
