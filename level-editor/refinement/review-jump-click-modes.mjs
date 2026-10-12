// Keep visible and hidden-floor selection separate while checking identical inputs.
import fs from "node:fs/promises";
import path from "node:path";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";

const directories = process.argv.slice(2);
assert.equal(directories.length, 2, "Supply normal and hidden supported-click audit directories");
const key = (file, probe) => JSON.stringify([file, probe.line, probe.start, probe.goal]);
const audits = await Promise.all(
  directories.map(async (directory, mode) => {
    const manifest = JSON.parse(await fs.readFile(path.join(directory, "diagnostics.json")));
    assert.ok(manifest.complete && manifest.supportedFloorClicks);
    const requested = new Set();
    const hashes = {};
    for (const entry of manifest.results) {
      assert.equal(path.basename(entry.file), entry.file);
      hashes[entry.file] = createHash("sha256")
        .update(await fs.readFile(path.join(directory, entry.file)))
        .digest("hex");
      for (const probe of entry.click_probes) {
        assert.equal(
          probe.hidden === true,
          mode === 1,
          "Each input must use its stated selection mode",
        );
        const id = key(entry.file, probe);
        assert.ok(!requested.has(id), "Duplicate requested click");
        requested.add(id);
      }
    }
    const report = JSON.parse(
      await fs.readFile(path.join(directory, "actor-supported-floor-click-report.json")),
    );
    const results = new Map();
    for (const result of report.results) {
      const id = key(result.file, result.probe);
      assert.ok(requested.has(id) && !results.has(id), "Unexpected or duplicate native result");
      assert.equal(result.probe.hidden === true, mode === 1);
      assert.equal(typeof result.passed, "boolean");
      results.set(id, result);
    }
    assert.ok(
      requested.size && results.size === requested.size,
      "Native audit omitted requested clicks",
    );
    return { hashes, results };
  }),
);
const [normal, hidden] = audits;
assert.deepEqual(normal.hashes, hidden.hashes, "Both audits must use identical descriptors");
assert.deepEqual([...normal.results.keys()].sort(), [...hidden.results.keys()].sort());
const results = [...normal.results].map(([id, result]) => {
  const behind = hidden.results.get(id);
  return {
    file: result.file,
    probe: result.probe,
    normal: result.passed,
    hidden: behind.passed,
    normalError: result.error,
    hiddenError: behind.error,
  };
});
const counts = {
  total: results.length,
  normal: results.filter((result) => result.normal).length,
  hiddenOnly: results.filter((result) => !result.normal && result.hidden).length,
  unreachable: results.filter((result) => !result.normal && !result.hidden).length,
};
const report = {
  complete: counts.unreachable === 0,
  scope: "native-normal-or-hidden-floor-click-coverage-not-normal-only-or-rendering-proof",
  directories,
  descriptorHashes: normal.hashes,
  ...counts,
  results,
};
await fs.writeFile(
  path.join(directories[0], "jump-click-modes-report.json"),
  JSON.stringify(report),
);
console.log(JSON.stringify(counts));
if (!report.complete) process.exitCode = 1;
