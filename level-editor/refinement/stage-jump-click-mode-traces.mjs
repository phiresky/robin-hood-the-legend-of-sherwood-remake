// Preserve successful normal/hidden traces after reviewing identical click inputs.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { createHash } from "node:crypto";

const sources = process.argv.slice(2);
assert.equal(sources.length, 2, "Supply reviewed normal and hidden click directories");
const review = JSON.parse(await fs.readFile(path.join(sources[0], "jump-click-modes-report.json")));
assert.equal(review.complete, true);
assert.equal(review.unreachable, 0);
assert.deepEqual(
  review.directories.map((value) => path.resolve(value)),
  sources.map((value) => path.resolve(value)),
);
const key = (result) =>
  JSON.stringify([result.file, result.probe.line, result.probe.start, result.probe.goal]);
const reports = await Promise.all(
  sources.map(async (directory, mode) => {
    const report = JSON.parse(
      await fs.readFile(path.join(directory, "actor-supported-floor-click-report.json")),
    );
    const results = new Map();
    for (const result of report.results) {
      assert.equal(result.probe.hidden === true, mode === 1);
      assert.ok(!results.has(key(result)));
      results.set(key(result), result);
    }
    assert.equal(results.size, review.total);
    return results;
  }),
);
const chosen = review.results.map((entry) => {
  const normal = reports[0].get(key(entry)),
    hidden = reports[1].get(key(entry));
  assert.ok(normal && hidden);
  assert.equal(normal.passed, entry.normal);
  assert.equal(hidden.passed, entry.hidden);
  const result = normal.passed ? normal : hidden;
  assert.equal(result.passed, true);
  assert.ok(result.arrival?.trajectory?.length, "Record ROBIN_TRACE_JUMP=1 for both modes");
  return result;
});
const manifest = JSON.parse(await fs.readFile(path.join(sources[0], "diagnostics.json")));
assert.equal(manifest.complete, true);
assert.equal(manifest.supportedFloorClicks, true);
const inputs = manifest.results.flatMap((entry) =>
  entry.click_probes.map((probe) => key({ file: entry.file, probe })),
);
assert.deepEqual(inputs.toSorted(), chosen.map(key).toSorted());
const bytes = new Map();
for (const entry of manifest.results) {
  assert.equal(path.basename(entry.file), entry.file);
  const [normal, hidden] = await Promise.all(
    sources.map((source) => fs.readFile(path.join(source, entry.file))),
  );
  assert.deepEqual(normal, hidden);
  assert.equal(
    createHash("sha256").update(normal).digest("hex"),
    review.descriptorHashes[entry.file],
  );
  bytes.set(entry.file, normal);
  entry.click_probes = chosen
    .filter((result) => result.file === entry.file)
    .map((result) => result.probe);
}
const output = await fs.mkdtemp("work/needs-review/map-compile/selected-jump-mode-traces-");
manifest.scope = "reviewed-normal-or-explicit-hidden-native-traces-not-rendering-proof";
manifest.modeSources = sources;
for (const [file, contents] of bytes) await fs.writeFile(path.join(output, file), contents);
await fs.writeFile(path.join(output, "diagnostics.json"), JSON.stringify(manifest));
await fs.writeFile(
  path.join(output, "actor-supported-floor-click-report.json"),
  JSON.stringify({
    complete: true,
    scope: manifest.scope,
    modeSources: sources,
    results: chosen,
  }),
);
console.log(output);
