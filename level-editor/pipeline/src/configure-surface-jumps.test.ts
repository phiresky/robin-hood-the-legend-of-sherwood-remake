import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { createHash } from "node:crypto";
import { surfaceJumpCompilerFixture } from "../../shared/test-fixtures/asset-gameplay.ts";
import { configureSurfaceJumps, configureAssetGameplay } from "./configure-surface-jumps.ts";

async function fixture() {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "surface-jump-publication-"));
  const library = path.join(root, "library");
  const { hut } = surfaceJumpCompilerFixture();
  const surface = hut.gameplay!.surfaces.find((surface) => surface.id === "west")!;
  const rules = structuredClone(surface.jump!);
  delete surface.jump;
  const before = JSON.stringify({ ...hut, authoringNote: "preserved" }) + "\n";
  const hash = (bytes: string) => createHash("sha256").update(bytes).digest("hex");
  const digest = hash(before);
  await fs.mkdir(path.join(library, "3d-assets/hut"), { recursive: true });
  await fs.mkdir(path.join(library, "scenes"));
  const descriptor = path.join(library, "3d-assets/hut/asset.json");
  await fs.writeFile(descriptor, before);
  const entry = {
    id: hut.id,
    name: hut.name,
    source_map: hut.source_map,
    descriptor: "hut/asset.json",
    model: `hut/${hut.model}`,
    descriptor_sha256: digest,
    editor: hut,
  };
  await fs.writeFile(
    path.join(library, "3d-assets/index.json"),
    JSON.stringify({ version: 1, assets: [entry] }),
  );
  const ref = { descriptor: "3d-assets/hut/asset.json", descriptor_sha256: digest };
  const document = {
    assetSources: [ref],
    sceneAssets: [ref],
    placements: [{ id: "keep", x: 123 }],
  };
  const scene = path.join(library, "scenes/map.rhlos-map.json");
  await fs.writeFile(scene, JSON.stringify(document));
  return {
    root,
    library,
    descriptor,
    scene,
    before,
    hash,
    document,
    edits: [{ asset: hut.id, surface: surface.id, descriptorSha256: digest, rules }],
  };
}

test("gameplay publication validates part appearance ownership and preserves geometry", async () => {
  const f = await fixture();
  try {
    const descriptor = JSON.parse(f.before);
    const edit = {
      asset: descriptor.id,
      descriptorSha256: f.hash(f.before),
      gameplay: descriptor.gameplay,
      partAppearances: { missing: { show: ["activate"] } },
    };
    await assert.rejects(
      configureAssetGameplay(f.library, [edit], path.join(f.root, "invalid"), true),
      /Missing appearance part/,
    );
    assert.equal(await fs.readFile(f.descriptor, "utf8"), f.before);
    const node = descriptor.parts[0].node;
    const reviewed = { ...edit, partAppearances: { [node]: { show: ["activate"] } } };
    await configureAssetGameplay(f.library, [reviewed], path.join(f.root, "published"), true);
    const after = JSON.parse(await fs.readFile(f.descriptor, "utf8"));
    assert.deepEqual(after.parts[0], {
      ...descriptor.parts[0],
      appearance: { show: ["activate"] },
    });
    assert.deepEqual(after.gameplay, descriptor.gameplay);
  } finally {
    await fs.rm(f.root, { recursive: true, force: true });
  }
});

test("surface authoring stages reviewable data then updates every scene pin without changing placements", async () => {
  const f = await fixture();
  try {
    const report = await configureSurfaceJumps(f.library, f.edits, path.join(f.root, "review"));
    assert.equal(report.applied, false);
    assert.equal(await fs.readFile(f.descriptor, "utf8"), f.before);
    await configureSurfaceJumps(f.library, f.edits, path.join(f.root, "applied"), true);
    const after = await fs.readFile(f.descriptor, "utf8");
    const changed = JSON.parse(after),
      original = JSON.parse(f.before);
    assert.deepEqual(changed.parts, original.parts);
    assert.equal(changed.model, original.model);
    assert.equal(changed.authoringNote, "preserved");
    const scene = JSON.parse(await fs.readFile(f.scene, "utf8"));
    assert.deepEqual(scene.placements, f.document.placements);
    for (const ref of [...scene.assetSources, ...scene.sceneAssets])
      assert.equal(ref.descriptor_sha256, f.hash(after));
    await assert.rejects(
      configureSurfaceJumps(f.library, f.edits, path.join(f.root, "stale"), true),
      /Stale reviewed/,
    );
    assert.equal(await fs.readFile(f.descriptor, "utf8"), after);
  } finally {
    await fs.rm(f.root, { recursive: true, force: true });
  }
});

test("compact precision edits retain all other gameplay and update pinned scenes", async () => {
  const f = await fixture();
  try {
    const { rules: _rules, ...edit } = f.edits[0]!;
    await configureAssetGameplay(
      f.library,
      [{ ...edit, preserveMovementPrecision: true }],
      path.join(f.root, "precision"),
      true,
    );
    const after = await fs.readFile(f.descriptor, "utf8");
    const expected = JSON.parse(f.before);
    expected.gameplay.surfaces.find(
      (s: { id: string }) => s.id === edit.surface,
    ).preserveMovementPrecision = true;
    assert.deepEqual(JSON.parse(after), expected);
    const scene = JSON.parse(await fs.readFile(f.scene, "utf8"));
    assert.deepEqual(scene.placements, f.document.placements);
    for (const ref of [...scene.assetSources, ...scene.sceneAssets])
      assert.equal(ref.descriptor_sha256, f.hash(after));
  } finally {
    await fs.rm(f.root, { recursive: true, force: true });
  }
});

test("failed installation restores descriptors and removes only its temporary files", async (t) => {
  const f = await fixture();
  const rename = fs.rename;
  try {
    t.mock.method(fs, "rename", async (...args: Parameters<typeof fs.rename>) => {
      if (args[1] === f.scene) throw new Error("injected installation failure");
      return rename(...args);
    });
    await assert.rejects(
      configureSurfaceJumps(f.library, f.edits, path.join(f.root, "failed"), true),
      /injected/,
    );
    assert.equal(await fs.readFile(f.descriptor, "utf8"), f.before);
    await assert.rejects(fs.stat(f.scene + ".surface-jumps.tmp"), { code: "ENOENT" });
  } finally {
    t.mock.restoreAll();
    await fs.rm(f.root, { recursive: true, force: true });
  }
});

test("geometry-derived gameplay rejects changed model bytes with an unchanged descriptor", async () => {
  const f = await fixture();
  try {
    const descriptor = JSON.parse(f.before);
    const model = path.join(path.dirname(f.descriptor), descriptor.model);
    const edits = [
      {
        asset: descriptor.id,
        descriptorSha256: f.hash(f.before),
        modelSha256: f.hash("reviewed model"),
        gameplay: descriptor.gameplay,
      },
    ];
    await fs.writeFile(model, "changed model");
    await assert.rejects(
      configureAssetGameplay(f.library, edits, path.join(f.root, "stale"), true),
      /Stale reviewed model/,
    );
    assert.equal(await fs.readFile(f.descriptor, "utf8"), f.before);
    await fs.writeFile(model, "reviewed model");
    await configureAssetGameplay(f.library, edits, path.join(f.root, "applied"), true);
    assert.deepEqual(
      JSON.parse(await fs.readFile(f.descriptor, "utf8")).gameplay,
      descriptor.gameplay,
    );
  } finally {
    await fs.rm(f.root, { recursive: true, force: true });
  }
});

test("spline gameplay replacement is bound to the reviewed model as well as its descriptor", async () => {
  const f = await fixture();
  try {
    const descriptor = JSON.parse(f.before);
    const model = path.join(path.dirname(f.descriptor), descriptor.model);
    await fs.writeFile(model, "reviewed model");
    const gameplay = {
      ...descriptor.gameplay,
      spline: {
        bounds: { min: [0, 0, 0], max: [100, 20, 40] },
        frames: {},
        modelSha256: f.hash("reviewed model"),
      },
    };
    const edits = [{ asset: descriptor.id, descriptorSha256: f.hash(f.before), gameplay }];
    await fs.writeFile(model, "changed model");
    await assert.rejects(
      configureAssetGameplay(f.library, edits, path.join(f.root, "stale-model"), true),
      /Stale spline model/,
    );
    assert.equal(await fs.readFile(f.descriptor, "utf8"), f.before);
    await fs.writeFile(model, "reviewed model");
    await configureAssetGameplay(f.library, edits, path.join(f.root, "calibrated"), true);
    assert.deepEqual(JSON.parse(await fs.readFile(f.descriptor, "utf8")).gameplay, gameplay);
  } finally {
    await fs.rm(f.root, { recursive: true, force: true });
  }
});
