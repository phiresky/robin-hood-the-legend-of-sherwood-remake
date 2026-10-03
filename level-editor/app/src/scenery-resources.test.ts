import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { encode } from "fast-png";
import { unzipSync } from "fflate";
import { assetCompilerFixture } from "../../shared/test-fixtures/asset-gameplay.ts";
import { compileMap, packageCompiledMap } from "./map-compile.ts";
import { collectSceneryResources } from "./scenery-resources.ts";

function fixture() {
  const { document, assets, hut } = assetCompilerFixture();
  const root = "sprites/flame.rhs.d";
  hut.gameplay!.animations = [
    {
      id: "flame",
      node: "building-999",
      file: "editor-flame",
      profile: "burning",
      anchor: [10, 30, 20],
      center: [4, 6],
      active: true,
      forceDisplay: false,
      shadow: true,
      displayPolyline: [],
      resourceDirectory: root,
    },
  ];
  const manifest = {
    pixel_format: "rgba",
    profiles: [
      {
        name: "burning",
        width: 8,
        height: 8,
        center_x: 4,
        center_y: 6,
        rows: [
          {
            action_id: 0,
            action_done: 283,
            average_speed: 0,
            hotspot_x: 0,
            hotspot_y: 0,
            path: "idle",
            frames: [
              { file: "0.png", delay: 2, distance: 0, offset_x: 0, offset_y: 0, sound_id: 65535 },
              { file: "1.png", delay: 4, distance: 0, offset_x: 1, offset_y: -1, sound_id: 65535 },
            ],
          },
        ],
      },
    ],
  };
  const files = new Map([
    [`${root}/manifest.json`, new TextEncoder().encode(JSON.stringify(manifest))],
    [
      `${root}/idle/0.png`,
      encode({ width: 8, height: 8, channels: 4, data: new Uint8Array(256).fill(255) }),
    ],
    [
      `${root}/idle/1.png`,
      encode({
        width: 8,
        height: 8,
        channels: 4,
        data: Uint8Array.from({ length: 256 }, (_, i) => (i % 4 === 3 ? 255 : 64)),
      }),
    ],
  ]);
  hut.resources = [...files].map(([path, bytes]) => ({
    path,
    sha256: createHash("sha256").update(bytes).digest("hex"),
  }));
  const compiled = compileMap(document, [0, 0, 600, 600], assets);
  const read = async (path: string) => {
    const bytes = files.get(path);
    if (!bytes) throw new Error(`missing ${path}`);
    return bytes;
  };
  return { assets, hut, files, compiled, read };
}

test("pinned scenery manifest and frames survive ZIP packaging exactly", async () => {
  const { assets, files, compiled, read } = fixture();
  const progress: number[] = [];
  const resources = await collectSceneryResources(compiled, assets, read, (done) =>
    progress.push(done),
  );
  assert.deepEqual(progress, [0, 1, 2, 3]);
  const zip = unzipSync(
    await packageCompiledMap(
      compiled,
      { color: new Uint8Array(600 * 600 * 4), depth: new Uint16Array(600 * 600) },
      [],
      resources,
    ),
  );
  for (const [path, bytes] of files)
    assert.deepEqual(zip[path.replace("sprites/flame", "Data/Animations/Day/editor-flame")], bytes);
  assert.equal(compiled.descriptor.asset_geometry!.animations!.length, 1);
  if (process.env.SCENERY_TEST_EXPORT_DIR) {
    const root = process.env.SCENERY_TEST_EXPORT_DIR;
    await mkdir(root);
    for (const [name, bytes] of Object.entries(zip)) {
      const destination = path.join(root, name);
      await mkdir(path.dirname(destination), { recursive: true });
      await writeFile(destination, bytes);
    }
  }
});

test("changed or missing scenery files omit only their animation", async () => {
  for (const failure of ["changed", "missing", "center", "unlisted"] as const) {
    const { assets, hut, files, compiled, read } = fixture();
    const geometry = structuredClone(compiled.descriptor.asset_geometry);
    if (failure === "changed") files.get("sprites/flame.rhs.d/idle/0.png")![0] = 0;
    if (failure === "missing") files.delete("sprites/flame.rhs.d/idle/0.png");
    if (failure === "center") hut.gameplay!.animations![0]!.center[0]++;
    if (failure === "unlisted") hut.resources!.pop();
    assert.deepEqual(await collectSceneryResources(compiled, assets, read), {});
    assert.deepEqual(compiled.descriptor.asset_geometry!.animations, []);
    assert.deepEqual(
      compiled.descriptor.asset_geometry!.sight_obstacles,
      geometry!.sight_obstacles,
    );
    assert.ok(
      compiled.warnings.some((warning) => warning.includes("Scenery bank editor-flame omitted")),
    );
  }
});

test("shared-bank scenery needs no library resource reads", async () => {
  const { assets, hut, compiled } = fixture();
  delete hut.gameplay!.animations![0]!.resourceDirectory;
  assert.deepEqual(
    await collectSceneryResources(compiled, assets, async () => {
      throw new Error("unexpected read");
    }),
    {},
  );
  assert.equal(compiled.descriptor.asset_geometry!.animations!.length, 1);
});

test("resource progress cancellation escapes best-effort handling immediately", async () => {
  const { assets, compiled, read } = fixture();
  const warnings = [...compiled.warnings];
  await assert.rejects(
    collectSceneryResources(compiled, assets, read, (done) => {
      if (done === 1) throw new Error("cancelled");
    }),
    /cancelled/,
  );
  assert.deepEqual(compiled.warnings, warnings);
  assert.equal(compiled.descriptor.asset_geometry!.animations!.length, 1);
});
