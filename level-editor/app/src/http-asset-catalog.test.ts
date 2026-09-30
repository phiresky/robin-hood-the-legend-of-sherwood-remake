import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { loadHttpAssetCatalog, loadHttpModelParts } from "./http-asset-catalog.ts";

test("model parts reconstruct exact bytes and reject corruption, truncation and invalid paths", async (t) => {
  const chunks = [new Uint8Array([1, 2, 3]), new Uint8Array([4, 5])];
  const hash = (data: Uint8Array) => createHash("sha256").update(data).digest("hex");
  const full = new Uint8Array([1, 2, 3, 4, 5]);
  const parts = chunks.map((data) => ({
    path: `3d-assets/model-chunk-${hash(data)}.bin`,
    sha256: hash(data),
  }));
  const model = { bytes: full.length, sha256: hash(full), parts };
  let corrupt = false;
  t.mock.method(globalThis, "fetch", async (url: string) => {
    const index = parts.findIndex((part) => "/library/" + part.path === url);
    assert.ok(index >= 0);
    return new Response(corrupt ? new Uint8Array([0]) : chunks[index]);
  });
  assert.deepEqual(await loadHttpModelParts("/library/", model), full);
  await assert.rejects(loadHttpModelParts("/library/", { ...model, bytes: 6 }), /size mismatch/);
  await assert.rejects(
    loadHttpModelParts("/library/", { ...model, parts: [{ ...parts[0]!, path: "../bad" }] }),
    /Invalid library model part/,
  );
  corrupt = true;
  await assert.rejects(loadHttpModelParts("/library/", model), /hash mismatch/);
});

test("HTTP catalog batches preserve complete embedded gameplay and validate their hashes", async (t) => {
  const assets = [{ id: "navigation", editor: { gameplay: { surfaces: ["surface"] } } }];
  const payload = JSON.stringify({ version: 1, assets });
  const sha256 = createHash("sha256").update(payload).digest("hex");
  const path = `3d-assets/catalog-${sha256}.json`;
  const manifest = { version: 1, assets: [], asset_shards: [{ path, sha256 }] };
  let corrupt = false;
  t.mock.method(globalThis, "fetch", async (url: string) => {
    if (url === "/library/3d-assets/index.json") return Response.json(manifest);
    assert.equal(url, "/library/" + path);
    return new Response(corrupt ? payload + " " : payload);
  });
  assert.deepEqual(await loadHttpAssetCatalog("/library/"), { version: 1, assets });
  corrupt = true;
  await assert.rejects(loadHttpAssetCatalog("/library/"), /hash mismatch/);
});

test("unsharded HTTP catalogs remain supported", async (t) => {
  const catalog = { version: 1, assets: [{ id: "house" }] };
  t.mock.method(globalThis, "fetch", async () => Response.json(catalog));
  assert.deepEqual(await loadHttpAssetCatalog("/library/"), catalog);
});
