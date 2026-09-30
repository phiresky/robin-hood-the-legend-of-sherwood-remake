import assert from "node:assert/strict";
import test from "node:test";
import { TexturePixelCache } from "./texture-pixel-cache.ts";

test("pixel cache enforces byte budget, promotes recent reads, and isolates mutable arrays", () => {
  const cache = new TexturePixelCache(8);
  const pixels = { data: new Uint8Array([1, 2, 3, 4]), width: 1, height: 1 };
  cache.set("a", pixels);
  pixels.data[0] = 99;
  cache.set("b", pixels);
  const read = cache.get("a")!;
  assert.equal(read.data[0], 1);
  read.data[0] = 77;
  cache.set("c", pixels);
  assert.equal(cache.get("b"), undefined);
  assert.equal(cache.get("a")!.data[0], 1);
  cache.set("a", { ...pixels, data: new Uint8Array(12) });
  assert.equal(cache.get("a"), undefined);
  assert.ok(cache.get("c"));
});
