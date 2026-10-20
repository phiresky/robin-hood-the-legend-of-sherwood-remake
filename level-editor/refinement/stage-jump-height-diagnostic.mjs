// Isolate authored flight height from projected walking contacts in native audits.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";

const [source, heightArgument] = process.argv.slice(2);
const height = Number(heightArgument);
assert.ok(source && heightArgument && Number.isFinite(height) && height > 0 && height <= 40);
const manifest = JSON.parse(await fs.readFile(path.join(source, "diagnostics.json")));
assert.equal(manifest.complete, true);
assert.equal(manifest.supportedFloorClicks, true);
for (const result of manifest.results) {
  assert.ok(result.diagnostic_jump_endpoints?.length);
  for (const edge of result.diagnostic_jump_endpoints) {
    assert.equal(edge.length, 2);
    for (const point of edge) {
      assert.ok(point.length === 3 && point.every(Number.isFinite));
      // These coordinates are projected x/y plus flight height, not world x/y/z.
      point[2] += height;
    }
  }
}
const output = await fs.mkdtemp("work/needs-review/map-compile/jump-height-diagnostic-");
for (const result of manifest.results) {
  assert.equal(path.basename(result.file), result.file);
  await fs.copyFile(path.join(source, result.file), path.join(output, result.file));
}
manifest.scope = "diagnostic-height-override-not-export-parity";
manifest.heightDiagnostic = { source, height, published: false };
await fs.writeFile(path.join(output, "diagnostics.json"), JSON.stringify(manifest));
console.log(output);
