import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import sharp from "sharp";
import { loadSwatch } from "./terrain.ts";
import { loadFxSprite } from "./fx.ts";
import { readAssetDescriptor, readLibraryIndex } from "./library.ts";
import { readTerrainSpec } from "./inputs.ts";
import { parseDetections } from "./reconstruct.ts";
import { parseTerrainSpec, parseAssetDescriptor } from "@rle/shared";

test("optional swatches reject broken files and read errors, and decode RGB", async (t) => {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), "swatch-input-"));
  t.after(() => fs.rm(dir, { recursive: true, force: true }));
  assert.equal(await loadSwatch("grass", dir), null);
  await fs.mkdir(path.join(dir, "grass"));
  const file = path.join(dir, "grass", "day.png");
  await fs.writeFile(file, "broken image");
  await assert.rejects(loadSwatch("grass", dir), /cannot decode terrain swatch/);
  await fs.rm(file);
  await fs.mkdir(file);
  await assert.rejects(loadSwatch("grass", dir), /cannot read image/);
  await fs.rmdir(file);
  await sharp({ create: { width: 2, height: 3, channels: 3, background: "white" } }).greyscale().png().toFile(file);
  const swatch = await loadSwatch("grass", dir);
  assert.equal(swatch?.data.length, 2 * 3 * 3);
});

test("FX absence is not cached and malformed or unreadable manifests fail", async (t) => {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), "fx-input-"));
  const previous = process.env.HACKABLE_DATADIR;
  process.env.HACKABLE_DATADIR = dir;
  t.after(async () => {
    if (previous === undefined) delete process.env.HACKABLE_DATADIR;
    else process.env.HACKABLE_DATADIR = previous;
    await fs.rm(dir, { recursive: true, force: true });
  });
  assert.equal(await loadFxSprite("Day", "bank", "sprite"), null);
  const bank = path.join(dir, "Data", "Animations", "Day", "bank.rhs.d");
  await fs.mkdir(bank, { recursive: true });
  const file = path.join(bank, "manifest.json");
  await assert.rejects(loadFxSprite("Day", "bank", "sprite"), /cannot read document/);
  await fs.writeFile(file, "{broken");
  await assert.rejects(loadFxSprite("Day", "bank", "sprite"), /invalid JSON/);
  await fs.writeFile(file, '{"profiles":[{}]}');
  await assert.rejects(loadFxSprite("Day", "bank", "sprite"), /invalid sprite profile/);
  await fs.writeFile(file, '{"profiles":[]}');
  assert.equal(await loadFxSprite("Day", "bank", "sprite"), null);
  await fs.writeFile(file, "null");
  await assert.rejects(loadFxSprite("Day", "bank", "sprite"), /invalid sprite manifest/);
  await fs.rm(file);
  await fs.mkdir(file);
  await assert.rejects(loadFxSprite("Day", "bank", "sprite"), /cannot read document/);
});

test("optional asset reads only treat absence as unreconstructed", async (t) => {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), "asset-input-"));
  t.after(() => fs.rm(dir, { recursive: true, force: true }));
  const file = path.join(dir, "asset.json");
  await assert.rejects(readLibraryIndex(dir, true), /Cannot read library index/);
  assert.equal(await readAssetDescriptor(file, false), undefined);
  await fs.writeFile(file, "{");
  await assert.rejects(readAssetDescriptor(file, false), /invalid JSON/);
  await fs.writeFile(file, '{"model":true}');
  await assert.rejects(readAssetDescriptor(file, false), /invalid asset descriptor/);
  await assert.rejects(readAssetDescriptor(dir, false), /cannot read document/);
});

test("terrain defaults require genuine absence, never JSON null or invalid structures", async (t) => {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), "terrain-input-"));
  t.after(() => fs.rm(dir, { recursive: true, force: true }));
  const file = path.join(dir, "terrain.json");
  assert.deepEqual(await readTerrainSpec(file), {});
  for (const value of [null, [], { roads: null }, { roads: [{ points: [], width: 0 }] }]) {
    await fs.writeFile(file, JSON.stringify(value));
    await assert.rejects(readTerrainSpec(file), /invalid terrain document/);
  }
  await fs.writeFile(file, "{");
  await assert.rejects(readTerrainSpec(file), /invalid JSON document/);
  await assert.rejects(readTerrainSpec(dir), /cannot read document/);
});

test("authored validators preserve extras and reject invalid transform input", () => {
  assert.deepEqual(parseTerrainSpec({ future: 1 }), { future: 1 });
  for (const value of [null, [], { roads: "bad" }, { roads: [{ points: [], width: -1 }] }, { walls: [{ asset: "wall", points: [], spacing: 0 }] }])
    assert.throws(() => parseTerrainSpec(value));
  assert.throws(() => parseAssetDescriptor({ id: "asset", source: {} }));
  assert.throws(() => parseDetections({ map: "York", apply_patches: true, detections: [{}] }));
  const detection = { id: "house", name: "House", prompt: "house", score: null, bbox: [0, 0, 1, 1], mask: "mask.png" };
  const doc = { map: "York", apply_patches: false, detections: [detection], future: 1 };
  assert.equal(parseDetections(doc), doc);
  assert.throws(() => parseDetections({ ...doc, detections: [detection, detection] }), /duplicate/);
});

test("asset validation accepts real transform shapes without stripping unknown metadata", () => {
  const desc = {
    id: "fixture", name: "Fixture", tags: [], scale_class: "unique",
    source: { map: "York", ambiance: "Day", bbox: [0, 0, 10, 20], extraction: { tool: "test" } },
    origin: [0, 0], anchor: [5, 20], images: { day: "day.png", mask: "mask.png" },
    volumes: { sight_obstacles: [{ opaque: true, solid: true, points: [{ x: 0, y: 1, z_bottom: 0, z_top: 2 }] }] },
    motion: { obstacles: [{ layer: 0, polygon: { points: [[1, 2], [3, 4]] } }], walkable: [] },
    model: {
      glb: "model.glb", textured: true, fit_iou: 0.8,
      bounds_local: { min: [0, 0, 0], max: [1, 1, 1] },
      placement: { asset: "fixture", position: [0, 0, 0], rotation: [0, 0, 0, 1], scale: 2 },
      pose_l2c: { rotation: [0, 0, 0, 1], translation: [0, 0, 0], scale: [1, 1, 1] },
      extraction: { tool: "test", crop: [0, 0, 10, 20] },
    },
    future: { opaque: "preserved" },
  };
  assert.equal(parseAssetDescriptor(desc), desc);
  for (const mutation of [
    { origin: [0] }, { source: { ...desc.source, bbox: [0, 0, 0, 20] } },
    { images: { mask: "mask.png" } }, { motion: { obstacles: null } },
    { model: { ...desc.model, placement: { ...desc.model.placement, scale: 0 } } },
    { model: { ...desc.model, pose_l2c: { rotation: [] } } },
  ]) assert.throws(() => parseAssetDescriptor({ ...desc, ...mutation }));
});
