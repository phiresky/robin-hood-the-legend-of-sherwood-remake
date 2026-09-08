import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import crypto from "node:crypto";
import {
  cachedArtifacts,
  contentKey,
  inspectCache,
  validateGlb,
} from "./provider-cache.ts";
import { reconstructWith } from "./backends.ts";
import { decodeRle, segment } from "./sam.ts";
import { reconstruct3d } from "./sam3d.ts";

async function temp(t: { after(fn: () => Promise<void>): void }) {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), "pipeline-cache-test-"));
  t.after(() => fs.rm(dir, { recursive: true, force: true }));
  return dir;
}
const validate = async (dir: string) => {
  const record = JSON.parse(
    await fs.readFile(path.join(dir, "response.json"), "utf8"),
  );
  if (record.value !== 42) throw new Error("bad response");
  return record.value as number;
};
test("content keys frame boundaries and include all parameters", () => {
  assert.notEqual(contentKey(["ab", "c"]), contentKey(["a", "bc"]));
  assert.notEqual(
    contentKey(["endpoint1", "image"]),
    contentKey(["endpoint2", "image"]),
  );
});
test("concurrent requests share one provider; complete cache is usable offline", async (t) => {
  const dir = path.join(await temp(t), "key");
  let calls = 0;
  const provider = async (staging: string) => {
    calls++;
    assert.equal((await inspectCache(dir, validate)).state, "miss");
    await fs.writeFile(
      path.join(staging, "response.json"),
      JSON.stringify({ value: 42 }),
    );
  };
  assert.deepEqual(
    await Promise.all(
      Array.from({ length: 8 }, () => cachedArtifacts(dir, validate, provider)),
    ),
    Array(8).fill(42),
  );
  assert.equal(calls, 1);
  assert.equal(
    await cachedArtifacts(
      dir,
      validate,
      async () => {
        throw new Error("must not call provider");
      },
      { offline: true },
    ),
    42,
  );
  await fs.writeFile(
    path.join(dir, "response.json"),
    JSON.stringify({ value: 42, changed: true }),
  );
  assert.equal((await inspectCache(dir, validate)).state, "corrupt");
  await assert.rejects(cachedArtifacts(dir, validate, provider), /corrupt/);
  assert.equal(calls, 1);
});
test("offline misses and interrupted downloads never become remote retries", async (t) => {
  const dir = path.join(await temp(t), "key");
  let calls = 0;
  const provider = async (staging: string) => {
    calls++;
    await fs.writeFile(path.join(staging, "response.json"), '{"value":42}');
    throw new Error("download failed after paid response");
  };
  await assert.rejects(
    cachedArtifacts(dir, validate, provider, { offline: true }),
    /offline.*miss/,
  );
  assert.equal(calls, 0);
  await assert.rejects(
    cachedArtifacts(dir, validate, provider),
    /download failed/,
  );
  assert.equal((await inspectCache(dir, validate)).state, "corrupt");
  await assert.rejects(cachedArtifacts(dir, validate, provider), /corrupt/);
  assert.equal(calls, 1);
  assert.equal(
    JSON.parse(await fs.readFile(path.join(dir, "response.json"), "utf8"))
      .value,
    42,
  );
});
test("invalid provider artifacts fail before atomic publication", async (t) => {
  const dir = path.join(await temp(t), "key");
  await assert.rejects(
    cachedArtifacts(dir, validate, async (staging) => {
      await fs.writeFile(path.join(staging, "response.json"), "not JSON");
    }),
  );
  await assert.rejects(fs.stat(path.join(dir, "complete.json")), {
    code: "ENOENT",
  });
});
function glb(): Buffer {
  let json = JSON.stringify({ asset: { version: "2.0" } });
  json += " ".repeat((4 - (json.length % 4)) % 4);
  const bytes = Buffer.alloc(20 + json.length);
  bytes.write("glTF");
  bytes.writeUInt32LE(2, 4);
  bytes.writeUInt32LE(bytes.length, 8);
  bytes.writeUInt32LE(json.length, 12);
  bytes.writeUInt32LE(0x4e4f534a, 16);
  bytes.write(json, 20);
  return bytes;
}
test("GLB validation rejects interrupted downloads", async (t) => {
  const file = path.join(await temp(t), "model.glb");
  await fs.writeFile(file, glb());
  await validateGlb(file);
  await fs.writeFile(file, glb().subarray(0, 25));
  await assert.rejects(validateGlb(file), /truncated/);
});
test("production SAM and 3D adapters reuse legacy caches without credentials", async (t) => {
  const workDirectory = await temp(t);
  const image = Buffer.from("cached-image");
  const params = {
    return_multiple_masks: true,
    max_masks: 8,
    include_scores: true,
    include_boxes: true,
  };
  const key = crypto
    .createHash("sha256")
    .update(image)
    .update(JSON.stringify(params))
    .digest("hex")
    .slice(0, 24);
  await fs.mkdir(path.join(workDirectory, "sam-cache"));
  await fs.writeFile(
    path.join(workDirectory, "sam-cache", `${key}.json`),
    JSON.stringify({
      endpoint: "fal-ai/sam-3/image-rle",
      response: { rle: ["1 2"] },
    }),
  );
  const masks = await segment(
    { imagePng: image, width: 2, height: 2 },
    { workDirectory, offline: true },
  );
  assert.deepEqual([...masks[0]!.data], [255, 255, 0, 0]);

  const spec = {
    image_url: "<image>",
    seed: 42,
    resolution: "1024",
    decimation_target: 100000,
    texture_size: "2048",
  };
  const backendKey = crypto
    .createHash("sha256")
    .update(image)
    .update(JSON.stringify({ backend: "trellis2", params: spec }))
    .digest("hex")
    .slice(0, 24);
  const backendDir = path.join(workDirectory, "trellis2-cache", backendKey);
  await fs.mkdir(backendDir, { recursive: true });
  await fs.writeFile(
    path.join(backendDir, "response.json"),
    JSON.stringify({
      endpoint: "fal-ai/trellis-2",
      request_id: "fixture",
      response: { model_glb: { url: "https://example.invalid/model.glb" } },
    }),
  );
  await fs.writeFile(path.join(backendDir, "model.glb"), glb());
  assert.equal(
    (
      await reconstructWith("trellis2", image, 42, {
        workDirectory,
        offline: true,
      })
    ).requestId,
    "fixture",
  );

  const mask = Buffer.from("cached-mask");
  const samParams = { seed: 42, export_textured_glb: true, mask_count: 1 };
  const samKey = crypto
    .createHash("sha256")
    .update(image)
    .update(mask)
    .update(JSON.stringify(samParams))
    .digest("hex")
    .slice(0, 24);
  const samDir = path.join(workDirectory, "sam3d-cache", samKey);
  await fs.mkdir(samDir, { recursive: true });
  await fs.writeFile(
    path.join(samDir, "response.json"),
    JSON.stringify({
      endpoint: "fal-ai/sam-3/3d-objects",
      request_id: "fixture",
      response: {
        model_glb: { url: "https://example.invalid/model.glb" },
        metadata: [
          { rotation: [0, 0, 0, 1], translation: [0, 0, 0], scale: [1, 1, 1] },
        ],
      },
    }),
  );
  await fs.writeFile(path.join(samDir, "object-0.glb"), glb());
  assert.equal(
    (
      await reconstruct3d(
        { imagePng: image, maskPngs: [mask] },
        { workDirectory, offline: true },
      )
    ).objects.length,
    1,
  );
});
test("SAM RLE validates integer ranges and accepts an empty mask", () => {
  assert.deepEqual([...decodeRle("", 2, 2).data], [0, 0, 0, 0]);
  for (const rle of ["1 -1", "1.5 2", "1 Infinity", "4 2"])
    assert.throws(() => decodeRle(rle, 2, 2));
});
