import test from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import type { ProjectionAssetEntry } from "@rle/shared";
import { AssetPreviewCache, estimatePreviewBytes } from "./asset-preview-cache.ts";

const root = {} as FileSystemDirectoryHandle;
const entry = (id = "one"): ProjectionAssetEntry => ({
  id,
  name: id,
  source_map: "test",
  descriptor: `${id}.json`,
  model: `${id}.glb`,
});
function fixture() {
  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute("position", new THREE.Float32BufferAttribute([0, 0, 0, 1, 1, 1], 3));
  const texture = new THREE.DataTexture(new Uint8Array(16), 2, 2);
  const material = new THREE.MeshBasicMaterial({ map: texture });
  const asset = new THREE.Mesh(geometry, material);
  const disposed = [0, 0, 0];
  [geometry, material, texture].forEach((resource, i) =>
    resource.addEventListener("dispose", () => disposed[i]++),
  );
  return { asset, disposed };
}

test("preview cache coalesces loads and returns independent roots sharing owned resources", async () => {
  const f = fixture();
  let loads = 0;
  const cache = new AssetPreviewCache({
    load: async () => {
      loads++;
      return f.asset;
    },
  });
  const [a, b] = await Promise.all([cache.acquire(root, entry()), cache.acquire(root, entry())]);
  assert.equal(loads, 1);
  assert.notEqual(a.asset, b.asset);
  assert.equal((a.asset as THREE.Mesh).geometry, f.asset.geometry);
  a.asset.position.x = 12;
  assert.equal(b.asset.position.x, 0);
  a.release();
  a.release();
  b.release();
  const c = await cache.acquire(root, entry());
  assert.equal(loads, 1);
  assert.deepEqual(f.disposed, [0, 0, 0]);
  cache.dispose();
  assert.deepEqual(f.disposed, [0, 0, 0]);
  c.release();
  assert.deepEqual(f.disposed, [1, 1, 1]);
});

test("preview cache evicts least recently used idle entries and pins active leases", async () => {
  const fixtures = new Map<string, ReturnType<typeof fixture>[]>();
  const cache = new AssetPreviewCache({
    maxEntries: 2,
    load: async (_root, item) => {
      const f = fixture();
      fixtures.set(item.id, [...(fixtures.get(item.id) ?? []), f]);
      return f.asset;
    },
  });
  const a = await cache.acquire(root, entry("a"));
  const b = await cache.acquire(root, entry("b"));
  b.release();
  const c = await cache.acquire(root, entry("c"));
  assert.deepEqual(fixtures.get("b")![0].disposed, [1, 1, 1]);
  assert.deepEqual(fixtures.get("a")![0].disposed, [0, 0, 0]);
  c.release();
  a.release(); // a is the most recently used idle entry now.
  const d = await cache.acquire(root, entry("d"));
  assert.deepEqual(fixtures.get("c")![0].disposed, [1, 1, 1]);
  const again = await cache.acquire(root, entry("a"));
  assert.equal(fixtures.get("a")!.length, 1);
  d.release();
  again.release();
  cache.dispose();
});

test("preview cache applies byte budget and never evicts a live oversized preview", async () => {
  const f = fixture();
  const cache = new AssetPreviewCache({ maxBytes: 1, load: async () => f.asset });
  const a = await cache.acquire(root, entry());
  assert.deepEqual(f.disposed, [0, 0, 0]);
  a.release();
  assert.deepEqual(f.disposed, [1, 1, 1]);
  cache.dispose();
  assert.deepEqual(f.disposed, [1, 1, 1]);
});

test("preview cache distinguishes library identity and entry revisions", async () => {
  let loads = 0;
  const cache = new AssetPreviewCache({
    load: async () => {
      loads++;
      return fixture().asset;
    },
  });
  const a = await cache.acquire(root, entry());
  const b = await cache.acquire({} as FileSystemDirectoryHandle, entry());
  const c = await cache.acquire(root, { ...entry(), descriptor_sha256: "updated" });
  assert.equal(loads, 3);
  for (const lease of [a, b, c]) lease.release();
  cache.dispose();
});

test("failed preview loads are retried", async () => {
  let loads = 0;
  const cache = new AssetPreviewCache({
    load: async () => {
      if (++loads === 1) throw new Error("unavailable");
      return fixture().asset;
    },
  });
  await assert.rejects(cache.acquire(root, entry()), /unavailable/);
  const a = await cache.acquire(root, entry());
  assert.equal(loads, 2);
  a.release();
  cache.dispose();
});

test("clear retires pending loads without contaminating the next generation", async () => {
  const f = fixture();
  let resolve!: (asset: THREE.Object3D) => void;
  let loads = 0;
  const cache = new AssetPreviewCache({
    load: async () => {
      if (++loads === 1)
        return new Promise<THREE.Object3D>((done) => {
          resolve = done;
        });
      return fixture().asset;
    },
  });
  const pending = cache.acquire(root, entry());
  await Promise.resolve();
  cache.clear();
  const next = await cache.acquire(root, entry());
  resolve(f.asset);
  await assert.rejects(pending, /cleared during loading/);
  assert.deepEqual(f.disposed, [1, 1, 1]);
  next.release();
  const reuse = await cache.acquire(root, entry());
  assert.equal(loads, 2);
  reuse.release();
  cache.dispose();
  await assert.rejects(cache.acquire(root, entry()), /disposed/);
});

test("dispose releases late in-flight results and prevents new loads", async () => {
  const f = fixture();
  let resolve!: (asset: THREE.Object3D) => void;
  const cache = new AssetPreviewCache({
    load: () =>
      new Promise((done) => {
        resolve = done;
      }),
  });
  const a = cache.acquire(root, entry());
  const b = cache.acquire(root, entry());
  await Promise.resolve();
  cache.dispose();
  resolve(f.asset);
  await Promise.all([assert.rejects(a, /cleared/), assert.rejects(b, /cleared/)]);
  assert.deepEqual(f.disposed, [1, 1, 1]);
});

test("memory estimate counts shared buffers and texture sources once plus GPU storage", () => {
  const f = fixture();
  const group = new THREE.Group();
  group.add(f.asset, f.asset.clone());
  assert.equal(estimatePreviewBytes(group), 24 * 2 + 16 * 2);
});
