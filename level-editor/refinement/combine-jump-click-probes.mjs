// Replay both versions' click probes against both exports, preserving line identities.
import fs from "node:fs/promises";
import path from "node:path";
import assert from "node:assert/strict";

const inputs = process.argv.slice(2);
assert.equal(inputs.length, 2, "Supply baseline and candidate supported-click directories");
const manifests = await Promise.all(
  inputs.map(async (directory) => {
    const manifest = JSON.parse(await fs.readFile(path.join(directory, "diagnostics.json")));
    assert.ok(manifest.complete && manifest.supportedFloorClicks);
    return manifest;
  }),
);
const entries = manifests.map(
  (manifest) => new Map(manifest.results.map((entry) => [entry.file, entry])),
);
assert.deepEqual([...entries[0].keys()].sort(), [...entries[1].keys()].sort());
const output = await fs.mkdtemp("work/map-compile/combined-jump-clicks-");
const results = [];
for (const label of ["baseline", "candidate"]) await fs.mkdir(path.join(output, label));
for (const file of entries[0].keys()) {
  assert.equal(path.basename(file), file, "Expected a local descriptor filename");
  const bytes = await Promise.all(
    inputs.map((directory) => fs.readFile(path.join(directory, file))),
  );
  const descriptors = bytes.map((bytes) => JSON.parse(bytes));
  assert.deepEqual(
    descriptors[0].asset_geometry.jump_line_pairs,
    descriptors[1].asset_geometry.jump_line_pairs,
    "Replay requires unchanged jump line identities and endpoints",
  );
  const probes = [
    ...new Map(
      entries
        .flatMap((entries) => entries.get(file).click_probes)
        .map((probe) => [JSON.stringify(probe), probe]),
    ).values(),
  ];
  assert.ok(probes.length && probes.every((probe) => probe && Number.isInteger(probe.line)));
  results.push({ file, click_probes: probes });
  for (const [i, label] of ["baseline", "candidate"].entries())
    await fs.writeFile(path.join(output, label, file), bytes[i]);
}
for (const label of ["baseline", "candidate"])
  await fs.writeFile(
    path.join(output, label, "diagnostics.json"),
    JSON.stringify({
      complete: true,
      supportedFloorClicks: true,
      scope: "same-native-clicks-across-selection-region-edits-not-rendering-proof",
      inputs,
      results,
    }),
  );
console.log(output);
console.log(
  JSON.stringify({ probes: results.reduce((sum, entry) => sum + entry.click_probes.length, 0) }),
);
