import fs from "node:fs/promises";
import assert from "node:assert/strict";

// Diagnostic ablation only: removing a hole is never a publishable floor fix.
const [source] = process.argv.slice(2);
assert.ok(source, "Provide church stair placement diagnostics");
const diagnostics = JSON.parse(await fs.readFile(`${source}/diagnostics.json`, "utf8"));
diagnostics.results = diagnostics.results.filter(
  (entry) => entry.file === "leicester-church-side-tower-0-0.level.json",
);
assert.equal(diagnostics.results.length, 1);
const output = await fs.mkdtemp("work/map-compile/church-hole-ablation-");
for (const entry of diagnostics.results) {
  const descriptor = JSON.parse(await fs.readFile(`${source}/${entry.file}`, "utf8"));
  assert.ok(entry.file.startsWith("leicester-church-side-tower-"));
  const landing = descriptor.asset_geometry.motion_data.layers[2];
  assert.equal(landing.length, 1);
  assert.equal(landing[0].obstacles.length, 1);
  assert.equal(landing[0].obstacles[0].state_id, 0);
  // Keep sector numbering stable while moving this blocker away from the route.
  landing[0].obstacles[0].polygon.points = [
    [0, 0],
    [1, 0],
    [1, 1],
    [0, 1],
  ];
  await fs.writeFile(`${output}/${entry.file}`, JSON.stringify(descriptor));
}
await fs.writeFile(
  `${output}/diagnostics.json`,
  JSON.stringify({
    ...diagnostics,
    scope: "diagnostic hole removal only; invalid for publication",
  }),
);
console.log(output);
