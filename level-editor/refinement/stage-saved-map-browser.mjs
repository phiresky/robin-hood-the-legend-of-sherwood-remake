// Snapshot a saved scene for full browser export/reopen verification.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import fs from "node:fs/promises";
import path from "node:path";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const [map, libraryArgument = "library", gameplayStage] = process.argv.slice(2);
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
assert.ok(map && /^[A-Za-z0-9_-]+$/.test(map), "Supply a saved map name");
const library = path.resolve(libraryArgument);
const filename = `${map}.rhlos-map.json`;
const bytes = await fs.readFile(path.join(library, "scenes", filename));
const document = JSON.parse(bytes);
assert.ok(!document.mission, "Map-only verification requires a scene without an authored mission");
assert.ok(document.assetSources?.length, "Saved scene has no pinned asset sources");
const links = ["3d-assets", "game-data", "population"];
for (const name of links) {
  assert.ok((await fs.stat(path.join(library, name))).isDirectory(), `Missing library ${name}`);
}
const output = await fs.mkdtemp(`work/needs-review/map-compile/${map}-rendered-library-`);
await fs.mkdir(path.join(output, "scenes"));
const replacements = new Map();
const edits = gameplayStage
  ? JSON.parse(await fs.readFile(path.join(gameplayStage, "edits.json")))
  : [];
const stagedAssets = [];
if (gameplayStage) {
  const catalog = JSON.parse(await fs.readFile(path.join(library, "3d-assets/index.json")));
  for (const edit of edits) {
    const source = document.assetSources.find((source) => source.id === edit.asset);
    assert.ok(source, `Staged asset is absent from ${map}: ${edit.asset}`);
    const original = await fs.readFile(path.join(library, source.descriptor));
    assert.equal(hash(original), edit.descriptorSha256);
    assert.equal(source.descriptor_sha256, edit.descriptorSha256);
    assert.equal(hash(await fs.readFile(path.join(library, source.model))), edit.modelSha256);
    const descriptor = JSON.parse(original);
    validateAssetGameplay(edit.gameplay, descriptor);
    descriptor.gameplay = edit.gameplay;
    const replacement = Buffer.from(JSON.stringify(descriptor));
    assert.ok(!replacements.has(source.descriptor), "Duplicate staged descriptor");
    replacements.set(source.descriptor, replacement);
    source.descriptor_sha256 = hash(replacement);
    const entry = catalog.assets.find((entry) => entry.id === edit.asset);
    assert.ok(entry, `Missing catalog entry: ${edit.asset}`);
    entry.descriptor_sha256 = source.descriptor_sha256;
    stagedAssets.push({
      id: edit.asset,
      before: edit.descriptorSha256,
      after: source.descriptor_sha256,
    });
  }
  replacements.set("3d-assets/index.json", Buffer.from(JSON.stringify(catalog)));
}
// The HTTP mount rejects nested symlinks escaping its asset root. A staged
// fixture therefore hardlinks unchanged asset files into private directories.
// Write replacements directly to fresh paths; never overwrite a shared inode.
async function linkTree(relative) {
  const target = path.join(output, relative);
  if (replacements.has(relative)) {
    await fs.writeFile(target, replacements.get(relative));
  } else if (gameplayStage && (relative === "3d-assets" || relative.startsWith("3d-assets/"))) {
    const source = await fs.realpath(path.join(library, relative));
    if ((await fs.stat(source)).isDirectory()) {
      await fs.mkdir(target);
      for (const name of await fs.readdir(source)) {
        if (!name.startsWith(".") && name !== "backups") await linkTree(`${relative}/${name}`);
      }
    } else {
      await fs.link(source, target);
    }
  } else await fs.symlink(path.join(library, relative), target);
}
const sceneBytes = gameplayStage ? Buffer.from(JSON.stringify(document)) : bytes;
await fs.writeFile(path.join(output, "scenes", filename), sceneBytes);
for (const name of links) await linkTree(name);
await fs.writeFile(
  path.join(output, "preparation.json"),
  JSON.stringify({
    scope: "full-saved-map-browser-fixture-not-render-or-gameplay-proof",
    sourceMap: map,
    sourceSceneSha256: hash(bytes),
    sceneSha256: hash(sceneBytes),
    ...(gameplayStage ? { gameplayStage, stagedAssets } : {}),
    pinnedAssetSources: document.assetSources.length,
    library,
  }),
);
console.log(output);
