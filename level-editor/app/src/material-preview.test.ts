import test from "node:test";
import { readFileSync } from "node:fs";
import { terrainMaterials } from "@rle/shared";
import atlas from "./terrain-textures/material-previews.json" with { type: "json" };
import assert from "node:assert/strict";
import { tintMaterialPreview } from "./material-preview.ts";

test("custom thumbnails retain source detail and alpha while using the chosen color", () => {
  const pixels = new Uint8ClampedArray([50, 50, 50, 123, 150, 150, 150, 255]);
  tintMaterialPreview(pixels, "#804020");
  assert.deepEqual([...pixels], [64, 32, 16, 123, 192, 96, 48, 255]);
});

test("custom preview tint rejects malformed colors and pixels", () => {
  assert.throws(() => tintMaterialPreview(new Uint8ClampedArray(4), "bad"));
  assert.throws(() => tintMaterialPreview(new Uint8ClampedArray(3), "#804020"));
});

test("prebuilt thumbnails cover the complete catalog and all custom bases", () => {
  assert.deepEqual(
    atlas.materials,
    terrainMaterials.map((m) => m.id),
  );
  assert.deepEqual(atlas.bases, ["grass", "dirt", "water", "paved"]);
  assert.equal(atlas.tileSize, 128);
  const png = readFileSync(new URL("./terrain-textures/material-previews.png", import.meta.url));
  assert.equal(png.readUInt32BE(16), atlas.columns * atlas.tileSize);
  assert.equal(
    png.readUInt32BE(20),
    Math.ceil((atlas.materials.length + atlas.bases.length) / atlas.columns) * atlas.tileSize,
  );
});
