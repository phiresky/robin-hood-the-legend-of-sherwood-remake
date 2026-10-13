// Restore the standalone hall from the assembly's asset-local definitions.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { translateGameplayFrames } from "../pipeline/src/translate-gameplay-frames.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { sceneToGame } from "../shared/src/geometry.ts";
import { insertProjectionAsset } from "../app/src/asset-commands.ts";
import { compileMap } from "../app/src/map-compile.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";

const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const read = async (directory, expected) => {
  const bytes = await fs.readFile(`library/3d-assets/${directory}/asset.json`);
  assert.equal(hash(bytes), expected, "Review changed authoring inputs");
  const descriptor = JSON.parse(bytes);
  const model = await fs.readFile(`library/3d-assets/${directory}/${descriptor.model}`);
  return { descriptor, descriptorSha256: hash(bytes), modelSha256: hash(model), model };
};
const target = await read(
  "nottingham/nottingham-castle-main-hall",
  "755e51ab7778392d271012f50ecc40fcdd8f6beccbfd372a9118f81ccfd1bdd1",
);
const owner = await read(
  "nottingham-castle-hall-and-watchtower",
  "f0f8b7693bc74144aea80d5e04c9b98a986c311d24f6b89c571b2da6438f6642",
);
assert.equal(target.descriptor.gameplay, undefined);
const nodes = new Set(target.descriptor.parts.map((part) => part.node));
const delta = sceneToGame(
  { kind: "oblique-orthographic", elevation_deg: 35 },
  owner.descriptor.source_origin_scene.map(
    (value, axis) => value - target.descriptor.source_origin_scene[axis],
  ),
);
for (const part of target.descriptor.parts) {
  const source = owner.descriptor.parts.find((row) => row.node === part.node);
  assert.ok(source);
  assert.equal(part.default_hidden, source.default_hidden);
  const a = source.obstacle_local_game,
    b = part.obstacle_local_game;
  assert.equal(a.points.length, b.points.length);
  for (const key of ["solid", "opaque", "mouse"]) assert.equal(a[key], b[key]);
  a.points.forEach((point, i) => {
    for (const [key, axis] of [
      ["x", 0],
      ["y", 1],
      ["z_bottom", 2],
      ["z_top", 2],
    ])
      assert.ok(
        Math.abs(point[key] + delta[axis] - b.points[i][key]) < 0.001,
        `${part.node}: non-translational geometry change`,
      );
  });
}
const source = structuredClone(owner.descriptor.gameplay);
const ownedArrays = new Set([
  "surfaces",
  "lights",
  "movementTransitions",
  "materials",
  "movementClearances",
  "doors",
  "lifts",
  "interiors",
]);
for (const [key, value] of Object.entries(source)) {
  if (ownedArrays.has(key)) source[key] = value.filter((row) => nodes.has(row.node));
  else
    assert.ok(
      ["version", "collision", "sightOrder", "movementSolids", "draft"].includes(key),
      `Unreviewed gameplay field: ${key}`,
    );
}
source.sightOrder = Object.fromEntries(
  Object.entries(source.sightOrder).filter(([node]) => nodes.has(node)),
);
source.movementSolids = source.movementSolids.filter((node) => nodes.has(node));
const gameplay = translateGameplayFrames(source, new Map([...nodes].map((node) => [node, delta])));
assert.equal(gameplay.surfaces.length, 5);
assert.equal(gameplay.interiors.length, 2);
assert.equal(gameplay.movementTransitions.length, 1);
const control = gameplay.movementTransitions[0];
assert.equal(control.id, "door-change-8");
assert.ok(
  [control.node, ...control.initialSight, ...control.appliedSight].every((node) => nodes.has(node)),
);
assert.deepEqual(control.appearances, ["nottingham-castle-main-hall/appearance-1"]);
control.appearances = ["appearance-1"];
const model = JSON.parse(target.model.subarray(20, 20 + target.model.readUInt32LE(12)));
const triggers = new Set(
  model.nodes.flatMap((node) => [
    ...(node.extras?.reveal_hide_when_applied ?? []),
    ...(node.extras?.reveal_show_when_applied ?? []),
    ...(node.extras?.reveal_material_patch ? [node.extras.reveal_material_patch] : []),
  ]),
);
assert.deepEqual([...triggers], control.appearances);
gameplay.draft.issues.push(
  "Standalone hall draft: translated from matching assembly parts; moved entrances, stairs and reveal behavior require native and rendered validation.",
  "The upper shaft entrance needs an independently placed receiving floor, such as the watchtower roof. No substitute floor is authored in this hall.",
  "Tower-owned lighting is excluded; independent hall lighting ownership remains under review.",
);
validateAssetGameplay(gameplay, target.descriptor);
const output = await fs.mkdtemp("work/needs-review/map-compile/nottingham-hall-gameplay-");
await fs.writeFile(
  `${output}/edits.json`,
  JSON.stringify([
    {
      asset: target.descriptor.id,
      descriptorSha256: target.descriptorSha256,
      modelSha256: target.modelSha256,
      gameplay,
    },
  ]),
);
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({
    scope: "unpublished-local-hall-restoration-not-full-parity",
    delta,
    ownerDescriptorSha256: owner.descriptorSha256,
    ownerModelSha256: owner.modelSha256,
    excludedParts: owner.descriptor.parts
      .filter((part) => !nodes.has(part.node))
      .map((part) => part.node),
    excludedLights: owner.descriptor.gameplay.lights,
  }),
);
console.log(output);
const results = [];
const descriptor = { ...target.descriptor, gameplay };
const reference = {
  id: descriptor.id,
  descriptor: "3d-assets/nottingham/nottingham-castle-main-hall/asset.json",
  model: "3d-assets/nottingham/nottingham-castle-main-hall/model.glb",
  descriptor_sha256: target.descriptorSha256,
  model_sha256: target.modelSha256,
  model_scene: descriptor.model_scene,
};
for (const height of [0, 40])
  for (const rotation of [0, 37, 90, 180]) {
    let document = {
      version: 1,
      map: `hall-${rotation}-${height}`,
      size: [3000, 3000],
      camera: { kind: "oblique-orthographic", elevation_deg: 35 },
      groups: [],
      objects: [],
      sceneAssets: [],
      assetSources: [],
      terrain: createTerrainGrid([0, 0, 3000, 3000], 3000, 0),
    };
    for (const x of [800, 1900]) {
      const inserted = insertProjectionAsset(document, descriptor, reference, [x, 1500, height]);
      document = inserted.document;
      document.groups.find((group) => group.id === inserted.selection.id).transform.rot_deg =
        rotation;
    }
    const compiled = compileMap(
      document,
      [0, 0, 3000, 3000],
      new Map([[descriptor.id, descriptor]]),
      { bestEffort: true },
    );
    const geometry = compiled.descriptor.asset_geometry;
    assert.equal(geometry.lifts.length, 2, "Each copy must retain its stairs");
    assert.equal(geometry.movement_transitions.length, 2, "Each copy needs its own reveal control");
    const file = `${rotation}-${height}.level.json`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
    await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
    results.push({
      file,
      rotation,
      height,
      warnings: geometry.warnings,
      buildings: geometry.buildings,
      movement_transitions: geometry.movement_transitions,
    });
  }
await fs.writeFile(
  `${output}/diagnostics.json`,
  JSON.stringify({
    scope: "two-copy-hall-compilation-on-flat-terrain-not-complete-entrance-parity",
    complete: true,
    results,
  }),
);
