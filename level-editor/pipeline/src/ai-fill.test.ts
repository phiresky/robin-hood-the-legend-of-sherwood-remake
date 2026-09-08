import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import sharp from "sharp";
import { prepare, callOpenAI, composite } from "./ai-fill.ts";

test("AI fill uses injected provider once, then validates cache without resolving credentials", async (t) => {
  const workDirectory = await fs.mkdtemp(
    path.join(os.tmpdir(), "ai-fill-test-"),
  );
  t.after(() => fs.rm(workDirectory, { recursive: true, force: true }));
  const original = Buffer.from([11, 22, 33, 255, 0, 0, 0, 0]);
  const tile = await sharp(original, {
    raw: { width: 2, height: 1, channels: 4 },
  })
    .png()
    .toBuffer();
  const prep = await prepare(tile);
  const png = await sharp({
    create: {
      width: 1024,
      height: 1024,
      channels: 4,
      background: { r: 100, g: 110, b: 120, alpha: 1 },
    },
  })
    .png()
    .toBuffer();
  let calls = 0;
  const request: typeof fetch = async () => {
    calls++;
    return new Response(
      JSON.stringify({ data: [{ b64_json: png.toString("base64") }] }),
      { status: 200 },
    );
  };
  const first = await callOpenAI(prep, "low", "default", null, {
    workDirectory,
    request,
    apiKey: () => "fake-provider",
  });
  assert.equal(first.cached, false);
  const second = await callOpenAI(prep, "low", "default", null, {
    workDirectory,
    offline: true,
    apiKey: () => {
      throw new Error("cache hit must not resolve credentials");
    },
    request: async () => {
      throw new Error("cache hit must not call provider");
    },
  });
  assert.equal(second.cached, true);
  assert.equal(calls, 1);
  const result = await composite(tile, prep, second.png);
  assert.deepEqual(result.rgba.subarray(0, 4), original.subarray(0, 4));
  assert.equal(result.rgba[7], 255);
});
