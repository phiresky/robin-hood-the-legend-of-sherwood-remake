import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import os from "node:os";
import { createHash } from "node:crypto";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { fileURLToPath } from "node:url";
import sharp from "sharp";
import {
  writeSceneryAnimationAssets,
  type SceneryAssetRecipe,
} from "./author-scenery-animation-assets.ts";
import { assetCompilerFixture } from "../../shared/test-fixtures/asset-gameplay.ts";
import type { GameplayAssetDescriptor } from "../../shared/src/asset-gameplay.ts";
import { compileMap } from "../../app/src/map-compile.ts";
import { collectSceneryResources } from "../../app/src/scenery-resources.ts";

const hash = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");
async function fixture() {
  const directory = "effects/flame.rhs.d";
  const manifest = {
    pixel_format: "rgba",
    profiles: [
      {
        name: "burning",
        width: 1,
        height: 1,
        center_x: 0,
        center_y: 0,
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
            ],
          },
        ],
      },
    ],
  };
  const png = await sharp({ create: { width: 1, height: 1, channels: 4, background: "red" } })
    .png()
    .toBuffer();
  const files = new Map<string, Uint8Array>([
    [`${directory}/manifest.json`, new TextEncoder().encode(JSON.stringify(manifest))],
    [`${directory}/idle/0.png`, png],
  ]);
  const recipe: SceneryAssetRecipe = {
    version: 1,
    entries: [
      {
        id: "test-fire",
        name: "Test fire",
        map: "authored",
        origin: [150, 250, 0],
        resources: [...files].map(([path, bytes]) => ({ path, sha256: hash(bytes) })),
        animations: [
          {
            id: "flame",
            anchor: [0, 0, 0],
            center: [0, 0],
            file: "fire",
            profile: "burning",
            active: true,
            forceDisplay: false,
            shadow: false,
            displayPolyline: [],
            resourceDirectory: directory,
          },
        ],
      },
    ],
  };
  const read = async (name: string) => {
    const bytes = files.get(name);
    if (!bytes) throw new Error(`Missing ${name}`);
    return bytes;
  };
  return { recipe, files, read };
}

test("scenery authoring CLI produces pinned assets that compile and repackage without source data", async (t) => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "scenery-author-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const { recipe, files } = await fixture();
  for (const [name, bytes] of files) {
    await fs.mkdir(path.dirname(path.join(root, "input", name)), { recursive: true });
    await fs.writeFile(path.join(root, "input", name), bytes);
  }
  await fs.writeFile(path.join(root, "recipe.json"), JSON.stringify(recipe));
  const output = path.join(root, "output");
  await promisify(execFile)(process.execPath, [
    fileURLToPath(new URL("./author-scenery-animation-assets.ts", import.meta.url)),
    "--recipe",
    path.join(root, "recipe.json"),
    "--library",
    path.join(root, "input"),
    "--out",
    output,
  ]);
  const generated: Awaited<ReturnType<typeof writeSceneryAnimationAssets>> = JSON.parse(
    await fs.readFile(path.join(output, "scenery-animation-assets.json"), "utf8"),
  );
  const { document, assets } = assetCompilerFixture();
  for (const source of generated.assetSources) {
    const bytes = await fs.readFile(path.join(output, source.descriptor));
    assert.equal(hash(bytes), source.descriptor_sha256);
    const descriptor: GameplayAssetDescriptor = JSON.parse(bytes.toString());
    assets.set(source.id, descriptor);
    assert.equal(hash(await fs.readFile(path.join(output, source.model))), source.model_sha256);
    assert.deepEqual(
      await fs.readFile(path.join(output, source.model)),
      await fs.readFile(path.join(output, "3d-assets/test-fire/lossy.glb")),
    );
  }
  document.assetSources!.push(...generated.assetSources);
  document.objects.push(...generated.objects);
  const compiled = compileMap(document, [0, 0, 600, 600], assets);
  const packaged = await collectSceneryResources(compiled, assets, (name) =>
    fs.readFile(path.join(output, name)),
  );
  assert.equal(compiled.descriptor.asset_geometry!.animations!.length, 1);
  assert.deepEqual(compiled.descriptor.asset_geometry!.animations![0]!.sprite, {
    frame_profile_name: "fire",
    profile_name: "burning",
    position_x: 150,
    position_y: 250,
    elevation: 0,
  });
  assert.equal(Object.keys(packaged).length, 2);
  assert.deepEqual(
    packaged["Data/Animations/Day/fire.rhs.d/idle/0.png"],
    files.get("effects/flame.rhs.d/idle/0.png"),
  );
});

test("invalid resources, profiles and duplicate assets fail before creating output", async (t) => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "scenery-author-invalid-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  for (const failure of ["hash", "frame", "profile", "duplicate", "path"]) {
    const { recipe, read } = await fixture();
    const entry = recipe.entries[0]!;
    if (failure === "hash") entry.resources![0]!.sha256 = "0".repeat(64);
    if (failure === "frame") entry.resources!.pop();
    if (failure === "profile") entry.animations[0]!.center[0] = 100;
    if (failure === "duplicate") recipe.entries.push(structuredClone(entry));
    if (failure === "path") entry.resources![0]!.path = "../escape";
    const output = path.join(root, failure);
    await assert.rejects(writeSceneryAnimationAssets(recipe, read, output));
    await assert.rejects(fs.stat(output), { code: "ENOENT" });
  }
  const { recipe, read } = await fixture();
  await assert.rejects(writeSceneryAnimationAssets(recipe, read, root), { code: "EEXIST" });
  assert.deepEqual(await fs.readdir(root), []);
});
