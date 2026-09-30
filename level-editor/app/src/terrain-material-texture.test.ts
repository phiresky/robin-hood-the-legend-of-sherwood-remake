import assert from "node:assert/strict";
import test from "node:test";
import { terrainMaterialTexture, terrainTexture } from "./terrain-texture.ts";

test("surface patterns are deterministic and differ from plain water beyond tint", () => {
  const still = terrainMaterialTexture("water_still");
  const rocks = terrainMaterialTexture("water_stones_large");
  const again = terrainMaterialTexture("water_stones_large");
  assert.deepEqual(rocks.image.data, again.image.data);
  assert.notDeepEqual(still.image.data, rocks.image.data);
  still.dispose();
  rocks.dispose();
  again.dispose();
});

test("pattern overlays preserve river ribbon edge alpha", () => {
  const material = terrainMaterialTexture("water_white", [], true);
  const base = terrainTexture("water", true);
  assert.equal(material.image.width, base.image.width);
  const data = material.image.data;
  for (let i = 3; i < data.length; i += 4) assert.equal(data[i], base.image.data[i]);
  material.dispose();
  base.dispose();
});

test("custom texture colors and unknown material failures remain explicit", () => {
  const texture = terrainMaterialTexture("red", [{ id: "red", name: "Red", color: "#ff0000" }]);
  const data = texture.image.data;
  for (let i = 0; i < data.length; i += 4) {
    assert.equal(data[i + 1], 0);
    assert.equal(data[i + 2], 0);
  }
  assert.throws(() => terrainMaterialTexture("missing"), /Unknown terrain material/);
  texture.dispose();
});

test("cached source pixels remain independently owned and custom edits invalidate their colors", () => {
  const first = terrainMaterialTexture("path_dirt", [], true);
  const expected = new Uint8Array(first.image.data);
  first.image.data.fill(0);
  first.dispose();
  const second = terrainMaterialTexture("path_dirt", [], true);
  assert.deepEqual(second.image.data, expected);
  const red = terrainMaterialTexture(
    "custom",
    [{ id: "custom", name: "Color", color: "#ff0000" }],
    true,
  );
  const green = terrainMaterialTexture(
    "custom",
    [{ id: "custom", name: "Color", color: "#00ff00" }],
    true,
  );
  assert.notDeepEqual(red.image.data, green.image.data);
  second.dispose();
  red.dispose();
  green.dispose();
});
