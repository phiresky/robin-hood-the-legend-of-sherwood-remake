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
  return { document, assets, hut, files, compiled, read, manifest };
}

test("unplaced and hidden assets cannot invalidate a placed scenery bank", async () => {
  for (const hidden of [false, true]) {
    const { document, assets, hut, read } = fixture();
    const unused = structuredClone(hut);
    unused.id = "unused-flame";
    unused.gameplay!.animations![0]!.profile = "unavailable-profile";
    unused.gameplay!.animations![0]!.resourceDirectory = "sprites/unavailable.rhs.d";
    assets.set(unused.id, unused);
    if (hidden) {
      document.assetSources!.push({ ...document.assetSources![0]!, id: unused.id });
      document.objects.push({
        ...structuredClone(document.objects[0]!),
        id: "unused-body",
        group: "unused",
        node: "asset:unused-flame:building-999",
      });
      document.groups.push({
        id: "unused",
        hidden: true,
        transform: { dx: 0, dy: 0, dz: 0, rot_deg: 0 },
      });
    }
    const compiled = compileMap(document, [0, 0, 600, 600], assets);
    assert.deepEqual(compiled.scenerySources, [{ assetId: hut.id, animationId: "flame" }]);
    const resources = await collectSceneryResources(compiled, assets, read);
    assert.equal(Object.keys(resources).length, 3);
    assert.equal(compiled.descriptor.asset_geometry!.animations!.length, 1);
    assert.ok(!compiled.warnings.some((warning) => warning.startsWith("Scenery bank")));
  }
});

test("an omitted animation cannot require a missing profile from a valid placed bank", async () => {
  const { document, assets, hut, read } = fixture();
  const invalid = structuredClone(hut.gameplay!.animations![0]!);
  invalid.id = "out-of-range";
  invalid.profile = "unavailable-profile";
  invalid.anchor[2] = 100000;
  hut.gameplay!.animations!.push(invalid);
  const compiled = compileMap(document, [0, 0, 600, 600], assets, { bestEffort: true });
  assert.equal(compiled.scenerySources.length, 1);
  assert.ok(compiled.warnings.some((warning) => warning.includes("out-of-range omitted")));
  assert.equal(Object.keys(await collectSceneryResources(compiled, assets, read)).length, 3);
  assert.equal(compiled.descriptor.asset_geometry!.animations!.length, 1);
});

test("separate placed assets share identical banks and namespace different frame contents", async () => {
  for (const conflict of ["same", "different", "broken", "shared"] as const) {
    const { document, assets, hut, read, files } = fixture();
    const copy = structuredClone(hut);
    copy.id = "second-flame";
    const root = "sprites/second.rhs.d";
    copy.gameplay!.animations![0]!.resourceDirectory = root;
    copy.resources = copy.resources!.map((pin) => ({
      ...pin,
      path: pin.path.replace("sprites/flame.rhs.d", root),
    }));
    for (const pin of copy.resources)
      files.set(pin.path, files.get(pin.path.replace(root, "sprites/flame.rhs.d"))!);
    if (conflict === "different") {
      const bytes = encode({
        width: 8,
        height: 8,
        channels: 4,
        data: new Uint8Array(256).fill(128),
      });
      files.set(`${root}/idle/0.png`, bytes);
      copy.resources[1]!.sha256 = createHash("sha256").update(bytes).digest("hex");
    }
    if (conflict === "broken") copy.resources[1]!.sha256 = "0".repeat(64);
    if (conflict === "shared") delete copy.gameplay!.animations![0]!.resourceDirectory;
    assets.set(copy.id, copy);
    document.assetSources!.push({ ...document.assetSources![0]!, id: copy.id });
    document.objects.push({
      ...structuredClone(document.objects[0]!),
      id: "second-body",
      group: "second",
      node: "asset:second-flame:building-999",
    });
    document.groups.push({ id: "second", transform: { dx: 200, dy: 0, dz: 0, rot_deg: 0 } });
    const compiled = compileMap(document, [0, 0, 900, 900], assets);
    const resources = await collectSceneryResources(compiled, assets, read);
    assert.equal(Object.keys(resources).length, conflict === "different" ? 6 : 3);
    const animations = compiled.descriptor.asset_geometry!.animations!;
    assert.equal(animations.length, conflict === "broken" ? 1 : 2);
    assert.equal(
      compiled.warnings.some((warning) => warning.includes("Scenery bank")),
      conflict === "broken",
    );
    for (const animation of animations) {
      const bank = animation.sprite.frame_profile_name;
      if (conflict === "shared" && bank === "editor-flame") continue;
      assert.ok(resources[`Data/Animations/Day/${bank}.rhs.d/manifest.json`]);
      if (conflict !== "same") assert.match(bank, /^editor-fx-[0-9a-f]{64}$/);
    }
    if (conflict === "different" || conflict === "shared")
      assert.notEqual(
        animations[0]!.sprite.frame_profile_name,
        animations[1]!.sprite.frame_profile_name,
      );
    if (conflict === "different") {
      const [a, b] = animations.map(
        (animation) =>
          resources[`Data/Animations/Day/${animation.sprite.frame_profile_name}.rhs.d/idle/0.png`],
      );
      assert.notDeepEqual(a, b);
      if (process.env.SCENERY_CONFLICT_EXPORT_DIR) {
        const root = process.env.SCENERY_CONFLICT_EXPORT_DIR;
        const zip = unzipSync(
          await packageCompiledMap(
            compiled,
            { color: new Uint8Array(900 * 900 * 4), depth: new Uint16Array(900 * 900) },
            [],
            resources,
          ),
        );
        await mkdir(root);
        for (const [name, bytes] of Object.entries(zip)) {
          const destination = path.join(root, name);
          await mkdir(path.dirname(destination), { recursive: true });
          await writeFile(destination, bytes);
        }
      }
    }
  }
});

test("pinned scenery manifest and frames survive ZIP packaging exactly", async () => {
  const { assets, files, compiled, read } = fixture();
  const progress: number[] = [];
  const resources = await collectSceneryResources(compiled, assets, read, (done) =>
    progress.push(done),
  );
  assert.deepEqual(progress, [0, 1, 2, 3, 3, 3]);
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

test("correctly pinned but invalid sprite data is omitted before export", async () => {
  for (const failure of [
    "action",
    "delay",
    "geometry",
    "empty",
    "directions",
    "png",
    "path",
    "format",
  ] as const) {
    const { assets, hut, files, compiled, read, manifest } = fixture();
    const profile = manifest.profiles[0]!;
    const row = profile.rows[0]!;
    if (failure === "action") row.action_id = 283;
    if (failure === "delay") row.frames[0]!.delay = -1;
    if (failure === "geometry") profile.width = 0;
    if (failure === "empty") row.frames = [];
    if (failure === "directions") {
      Object.assign(row, { direction: 0 });
      profile.rows.push(structuredClone(row));
    }
    if (failure === "png") files.set("sprites/flame.rhs.d/idle/0.png", new Uint8Array([1, 2, 3]));
    if (failure === "path") row.frames[0]!.file = "../manifest.json";
    if (failure === "format") manifest.pixel_format = "unknown";
    files.set(
      "sprites/flame.rhs.d/manifest.json",
      new TextEncoder().encode(JSON.stringify(manifest)),
    );
    hut.resources = [...files].map(([path, bytes]) => ({
      path,
      sha256: createHash("sha256").update(bytes).digest("hex"),
    }));
    assert.deepEqual(await collectSceneryResources(compiled, assets, read), {}, failure);
    assert.deepEqual(compiled.descriptor.asset_geometry!.animations, [], failure);
    assert.ok(
      compiled.warnings.some((warning) => warning.includes("Scenery bank editor-flame omitted")),
      failure,
    );
  }
});
