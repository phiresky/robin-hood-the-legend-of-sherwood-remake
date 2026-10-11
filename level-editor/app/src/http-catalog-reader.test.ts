import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { createHttpCatalogReader } from "./http-catalog-reader.ts";
import { readJson, setJsonReader } from "./fs.ts";

const hash = (data: string | Uint8Array) => createHash("sha256").update(data).digest("hex");

test("virtual catalog reads fetch once per read, preserve descriptors and refresh routing", async (t) => {
  let revision = 1;
  const requests: string[] = [];
  const versions = [new Uint8Array([1, 2]), new Uint8Array([3, 4])];
  t.mock.method(globalThis, "fetch", async (url: string, init?: RequestInit) => {
    requests.push(url);
    const bytes = versions[revision - 1]!;
    if (url.endsWith("index.json")) {
      assert.equal(init?.cache, "no-store");
      return Response.json({
        version: 1,
        assets: [
          { id: "tree", editor: { gameplay: { masks: [{ triangles: [[[1, 2, 3]]] }] }, revision } },
        ],
        model_shards: {
          "3d-assets/tree/model.glb": {
            bytes: bytes.length,
            sha256: hash(bytes),
            parts: [{ path: `3d-assets/model-chunk-${hash(bytes)}.bin`, sha256: hash(bytes) }],
          },
        },
      });
    }
    assert.equal(url, `/library/3d-assets/model-chunk-${hash(bytes)}.bin`);
    return new Response(bytes);
  });
  const reader = createHttpCatalogReader("/library/");
  assert.equal(requests.length, 0);
  const dir = {
    getFileHandle: () => {
      throw Error("Catalog file copy forbidden");
    },
  } as unknown as FileSystemDirectoryHandle;
  setJsonReader(dir, "index.json", reader.read);
  const first = await readJson<{ assets: { editor: { revision: number; gameplay: unknown } }[] }>(
    dir,
    "index.json",
  );
  assert.equal(first.assets[0]!.editor.revision, 1);
  assert.deepEqual(first.assets[0]!.editor.gameplay, { masks: [{ triangles: [[[1, 2, 3]]] }] });
  assert.deepEqual(await reader.readModel("3d-assets/tree/model.glb"), versions[0]);
  revision = 2;
  const second = await readJson<typeof first>(dir, "index.json");
  assert.equal(second.assets[0]!.editor.revision, 2);
  assert.equal(first.assets[0]!.editor.revision, 1);
  assert.deepEqual(await reader.readModel("3d-assets/tree/model.glb"), versions[1]);
  assert.equal(requests.filter((x) => x.endsWith("index.json")).length, 2);
});

test("direct model reads share lazy routing and retry failed initialization", async (t) => {
  let calls = 0;
  let fail = true;
  t.mock.method(globalThis, "fetch", async () => {
    calls++;
    await Promise.resolve();
    if (fail) return new Response("failed", { status: 503 });
    return Response.json({ version: 1, assets: [] });
  });
  const reader = createHttpCatalogReader("/library/");
  const first = await Promise.allSettled([reader.readModel("one"), reader.readModel("two")]);
  assert.ok(first.every((x) => x.status === "rejected"));
  assert.equal(calls, 1);
  fail = false;
  assert.deepEqual(await Promise.all([reader.readModel("one"), reader.readModel("two")]), [
    null,
    null,
  ]);
  assert.equal(calls, 2);
});

test("virtual reads preserve shard verification and reject corruption", async (t) => {
  const body = JSON.stringify({ version: 1, assets: [{ id: "full" }] });
  const sha256 = hash(body);
  let corrupt = false;
  t.mock.method(globalThis, "fetch", async (url: string) =>
    url.endsWith("index.json")
      ? Response.json({
          version: 1,
          assets: [],
          asset_shards: [{ path: `3d-assets/catalog-${sha256}.json`, sha256 }],
        })
      : new Response(corrupt ? body + " " : body),
  );
  const reader = createHttpCatalogReader("/library/");
  assert.deepEqual(await reader.read(), { version: 1, assets: [{ id: "full" }] });
  corrupt = true;
  await assert.rejects(reader.read(), /hash mismatch/);
});
