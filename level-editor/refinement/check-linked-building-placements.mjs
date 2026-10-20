// Check explicit cross-asset room links at every completed placement variant.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { execFileSync } from "node:child_process";

const [placements, stage, room] = process.argv.slice(2);
assert.ok(placements && stage && room, "Supply placements, gameplay stage and room ID");
const input = JSON.parse(await fs.readFile(`${placements}/diagnostics.json`));
assert.equal(input.complete, true);
assert.ok(input.results.length > 0);
const output = await fs.mkdtemp("work/needs-review/map-compile/linked-building-placements-");
console.log(output);
const results = [];
const save = (complete) =>
  fs.writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({
      scope: "moved-cross-asset-room-links-awaiting-native-verification",
      complete,
      placements,
      stage,
      room,
      results,
    }),
  );
await save(false);
for (const [index, row] of input.results.entries()) {
  const gallery = `${output}/input-${index}`;
  await fs.mkdir(gallery);
  await fs.copyFile(`${placements}/${row.file}.scene.json`, `${gallery}/scene.json`);
  const checked = execFileSync(
    process.execPath,
    ["refinement/check-linked-building-gallery.mjs", gallery, stage, room],
    { encoding: "utf8" },
  ).trim();
  assert.equal(path.dirname(checked), "work/needs-review/map-compile");
  const report = JSON.parse(await fs.readFile(`${checked}/diagnostics.json`));
  assert.equal(report.complete, true);
  assert.equal(report.results.length, 2);
  for (const variant of report.results) {
    const file = `${index}-${variant.file}`;
    for (const suffix of ["", ".scene.json"])
      await fs.copyFile(`${checked}/${variant.file}${suffix}`, `${output}/${file}${suffix}`);
    results.push({ ...variant, file });
  }
  await save(false);
}
await save(true);
