import test from "node:test";
import assert from "node:assert/strict";
import { TextureAreaFilter } from "./texture-area-filter.ts";

test("area filtering removes high-frequency detail instead of aliasing it", () => {
  const width = 8,
    height = 8;
  const data = new Uint8Array(width * height * 4);
  for (let y = 0; y < height; y++)
    for (let x = 0; x < width; x++) {
      const index = (y * width + x) * 4;
      data.fill((x + y) % 2 ? 255 : 0, index, index + 3);
      data[index + 3] = 255;
    }
  const filter = new TextureAreaFilter({ data, width, height }, 2);
  const row = new Float32Array(8);
  for (const center of [-0.25, 0, 0.3, 4, 7.8, 16]) {
    filter.sampleRow(center, 4, true, row);
    assert.deepEqual([...row], [127.5, 127.5, 127.5, 255, 127.5, 127.5, 127.5, 255]);
  }
});

test("fractional vertical footprints wrap repeated tiles and clamp isolated details", () => {
  const filter = new TextureAreaFilter(
    { width: 1, height: 2, data: new Uint8Array([0, 0, 0, 255, 200, 200, 200, 255]) },
    1,
  );
  const row = new Float32Array(4);
  filter.sampleRow(1, 1, true, row);
  assert.equal(row[0], 100);
  filter.sampleRow(0, 1, true, row);
  assert.equal(row[0], 100);
  filter.sampleRow(0, 1, false, row);
  assert.equal(row[0], 0);
  filter.sampleRow(2, 1, false, row);
  assert.equal(row[0], 200);
  filter.sampleRow(0.25, 1, true, row);
  assert.equal(row[0], 50);
  filter.sampleRow(3, 20, true, row);
  assert.equal(row[0], 100);
});

test("area averages premultiply alpha so invisible colors cannot bleed into shore edges", () => {
  const filter = new TextureAreaFilter(
    { width: 2, height: 1, data: new Uint8Array([255, 0, 0, 255, 0, 0, 255, 0]) },
    1,
  );
  const row = new Float32Array(4);
  filter.sampleRow(0.5, 1, false, row);
  assert.deepEqual([...row], [127.5, 0, 0, 127.5]);
  assert.equal((row[0]! * 255) / row[3]!, 255);
});
