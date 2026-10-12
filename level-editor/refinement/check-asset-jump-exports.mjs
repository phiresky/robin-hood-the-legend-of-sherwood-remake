// Isolate reusable asset jump definitions on authored flat ground.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createHash } from "node:crypto";
import { compileMap } from "../app/src/map-compile.ts";
import { insertProjectionAsset } from "../app/src/asset-commands.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";

const args = process.argv.slice(2);
const stages = args.filter((arg) => arg.startsWith("--stage="));
assert.ok(stages.length <= 1, "Supply at most one staged gameplay directory");
const stage = stages[0]?.slice("--stage=".length);
const rotationArgs = args.filter((arg) => arg.startsWith("--rotations="));
assert.ok(rotationArgs.length <= 1, "Supply at most one rotation list");
const rotations = rotationArgs.length
  ? rotationArgs[0].slice("--rotations=".length).split(",").map(Number)
  : [0, 37, 90, 180];
assert.ok(
  rotations.length &&
    rotations.every((value) => Number.isFinite(value) && value >= 0 && value < 360),
);
assert.equal(new Set(rotations).size, rotations.length, "Rotation samples must be unique");
const edits = stage ? JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8")) : [];
const ids = args.filter((arg) => !arg.startsWith("--stage=") && !arg.startsWith("--rotations="));
assert.ok(ids.length && ids.every((id) => !id.startsWith("-")));
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8"));
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const output = await fs.mkdtemp("work/map-compile/asset-jump-exports-");
console.log(output);
const results = [];
const save = (complete) =>
  fs.writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({
      scope: "static-geometry-only-not-gameplay-parity",
      ...(stage ? { gameplayStage: stage } : {}),
      complete,
      results,
    }),
  );
await save(false);
for (const id of ids) {
  const entry = index.assets.find((entry) => entry.id === id);
  assert.ok(entry, id);
  const bytes = await fs.readFile(`library/3d-assets/${entry.descriptor}`);
  assert.equal(hash(bytes), entry.descriptor_sha256);
  const descriptor = JSON.parse(bytes);
  const reference = {
    id,
    role: "objects",
    descriptor: `3d-assets/${entry.descriptor}`,
    descriptor_sha256: hash(bytes),
    model: `3d-assets/${entry.model}`,
    model_sha256: hash(await fs.readFile(`library/3d-assets/${entry.model}`)),
    model_scene: entry.model_scene,
    resources: descriptor.resources ?? [],
  };
  const edit = edits.find((edit) => edit.asset === id);
  if (edit) {
    assert.equal(edit.descriptorSha256, reference.descriptor_sha256);
    assert.equal(edit.modelSha256, reference.model_sha256);
    descriptor.gameplay = edit.gameplay;
  }
  for (const elevation of [0, 40])
    for (const rotation of rotations) {
      const empty = {
        version: 1,
        map: id,
        camera: { kind: "oblique-orthographic", elevation_deg: 35 },
        size: [2000, 2000],
        objects: [],
        groups: [],
        sceneAssets: [],
        assetSources: [],
        terrain: createTerrainGrid([0, 0, 2000, 2000], 2000, elevation),
      };
      const { document } = insertProjectionAsset(empty, descriptor, reference, [
        800,
        800,
        elevation,
      ]);
      document.groups[0].transform.rot_deg = rotation;
      const compiled = compileMap(document, [0, 0, 2000, 2000], new Map([[id, descriptor]]), {
        bestEffort: true,
      });
      const file = `${id}-${elevation}-${rotation}.level.json`;
      await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
      await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
      results.push({
        file,
        warnings: compiled.warnings,
        pairs: compiled.descriptor.asset_geometry.jump_line_pairs?.length ?? 0,
      });
      await save(false);
    }
  console.log(`${id}: ${2 * rotations.length} placements compiled`);
}
await save(true);
