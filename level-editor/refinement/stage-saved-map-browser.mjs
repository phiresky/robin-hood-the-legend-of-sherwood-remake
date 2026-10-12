// Snapshot a saved scene for full browser export/reopen verification.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import fs from "node:fs/promises";
import path from "node:path";

const [map, libraryArgument = "library"] = process.argv.slice(2);
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
const output = await fs.mkdtemp(`work/map-compile/${map}-rendered-library-`);
await fs.mkdir(path.join(output, "scenes"));
await fs.writeFile(path.join(output, "scenes", filename), bytes);
for (const name of links) await fs.symlink(path.join(library, name), path.join(output, name));
await fs.writeFile(
  path.join(output, "preparation.json"),
  JSON.stringify({
    scope: "full-saved-map-browser-fixture-not-render-or-gameplay-proof",
    sourceMap: map,
    sceneSha256: createHash("sha256").update(bytes).digest("hex"),
    pinnedAssetSources: document.assetSources.length,
    library,
  }),
);
console.log(output);
