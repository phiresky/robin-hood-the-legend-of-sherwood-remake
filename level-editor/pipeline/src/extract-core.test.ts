import test, { type TestContext } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import sharp from "sharp";
import { parseProtoLevel, type LibraryIndexEntry } from "@rle/shared";
import {
  EXTRACT_DEFAULTS,
  runExtraction,
  type ExtractionRuntime,
  type ExtractOptions,
} from "./extract-core.ts";
import { readLibraryIndex } from "./library.ts";

const options: ExtractOptions = {
  ...EXTRACT_DEFAULTS,
  map: "York",
  bbox: [0, 0, 4, 4],
  pad: 0,
  name: "Fixture",
  id: "fixture",
  tags: [],
};
const entry = (
  id: string,
  tags: string[] = [],
  source_map = "York",
): LibraryIndexEntry => ({
  id,
  name: id,
  tags,
  scale_class: "unique",
  source_map,
  bbox: [0, 0, 4, 4],
});

async function fixture(t: TestContext) {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "extract-index-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const libraryDirectory = path.join(root, "library");
  const workDirectory = path.join(root, "work");
  await fs.mkdir(libraryDirectory);
  const index = path.join(libraryDirectory, "index.json");
  let levelReads = 0;
  let providerCalls = 0;
  const writes: string[] = [];
  const image = await sharp({
    create: { width: 4, height: 4, channels: 3, background: "white" },
  })
    .png()
    .toBuffer();
  const runtime: ExtractionRuntime = {
    libraryDirectory,
    workDirectory,
    loadProtoLevel: async () => {
      levelReads++;
      return parseProtoLevel({
        format: "Fullgame",
        misc: {},
        sight_obstacles: [],
        patches: [],
        animations: [],
        material_sectors: [],
        light_sectors: [],
        elevation_lines: [],
        masks: [],
        sound_sources: [],
        jump_zones: [],
        jump_line_pairs: [],
        lifts: [],
        buildings: [],
        motion_data: { layers: [], graph_bytes: [] },
      });
    },
    mapImageSource: async () => image,
    segment: async () => {
      providerCalls++;
      return [
        {
          data: new Uint8Array(16).fill(255),
          width: 4,
          height: 4,
          score: 1,
          box: null,
        },
      ];
    },
    writeMaskedAsset: async (spec) => {
      writes.push(spec.id);
    },
  };
  return {
    runtime,
    index,
    writes,
    levelReads: () => levelReads,
    providerCalls: () => providerCalls,
  };
}

test("invalid existing indexes fail extraction before level loading or provider work", async (t) => {
  const { runtime, index, writes, providerCalls, levelReads } =
    await fixture(t);
  for (const invalid of [
    "{broken",
    "{}",
    "[null]",
    JSON.stringify([{ id: "bad" }]),
  ]) {
    await fs.writeFile(index, invalid);
    await assert.rejects(
      runExtraction(options, runtime),
      /Invalid library index/,
    );
    assert.equal(providerCalls(), 0);
    assert.equal(levelReads(), 0);
    assert.deepEqual(writes, []);
    assert.equal(await fs.readFile(index, "utf8"), invalid);
  }
});

test("read failures are not mistaken for an absent index", async (t) => {
  const { runtime, index, providerCalls } = await fixture(t);
  await fs.mkdir(index); // Deterministic I/O failure even when tests run as root.
  await assert.rejects(
    runExtraction(options, runtime),
    /Cannot read library index/,
  );
  assert.equal(providerCalls(), 0);
});

test("an absent index permits extraction and healthy entries retain overlap exclusions", async (t) => {
  const { runtime, index, writes, providerCalls } = await fixture(t);
  assert.deepEqual(await readLibraryIndex(runtime.libraryDirectory), []);
  const first = await runExtraction(options, runtime);
  assert.deepEqual(
    first.written.map((item) => item.id),
    ["fixture"],
  );
  for (const excluded of [
    entry("effect", ["fx"]),
    entry("patch", ["patch"]),
    entry("elsewhere", [], "Lincoln"),
  ]) {
    await fs.writeFile(index, JSON.stringify([excluded]));
    const result = await runExtraction(options, runtime);
    assert.equal(result.written.length, 1);
  }
  await fs.writeFile(index, JSON.stringify([entry("existing-building")]));
  const duplicate = await runExtraction(options, runtime);
  assert.equal(duplicate.written.length, 0);
  assert.match(duplicate.skipped[0]!.reason, /duplicate of existing-building/);
  assert.equal(providerCalls(), 5);
  assert.equal(writes.length, 4);
});

test("deduplication rereads the validated index after provider work without holding a lease", async (t) => {
  const { runtime, index, writes } = await fixture(t);
  const segment = runtime.segment;
  runtime.segment = async (...args) => {
    await assert.rejects(
      fs.stat(path.join(runtime.libraryDirectory, ".index.lock")),
      { code: "ENOENT" },
    );
    await fs.writeFile(
      index,
      JSON.stringify([entry("published-during-provider")]),
    );
    return segment(...args);
  };
  const result = await runExtraction(options, runtime);
  assert.match(
    result.skipped[0]!.reason,
    /duplicate of published-during-provider/,
  );
  assert.deepEqual(writes, []);
});

test("an index corrupted during provider work rejects before publishing any asset", async (t) => {
  const { runtime, index, writes } = await fixture(t);
  const segment = runtime.segment;
  runtime.segment = async (...args) => {
    await fs.writeFile(index, "{invalidated");
    return segment(...args);
  };
  await assert.rejects(
    runExtraction(options, runtime),
    /Invalid library index/,
  );
  assert.deepEqual(writes, []);
});
