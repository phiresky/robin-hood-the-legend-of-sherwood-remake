import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { compileMap } from "../app/src/map-compile.ts";

// Candidate authoring only: the wall owns its opening independently of the
// stair. Native traversal and mesh review are required before publication.
const [stage] = process.argv.slice(2);
assert.ok(stage);
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8"));
assert.equal(edits.length, 1);
assert.equal(edits[0].asset, "york-outer-east-upper-wall-stair");
const document = await readStoredMap("library/scenes/york.rhlos-map.json", "library");
const assets = await pinnedDescriptors("library", document.assetSources, document.sceneAssets);
const stair = edits[0].gameplay;
const floor = stair.surfaces.find((surface) => surface.id === stair.lifts[0].surface);
const placement = document.groups.find((group) => group.id === edits[0].asset).transform;
const id = "york-outer-east-upper-curtain-wall";
const descriptor = assets.get(id);
const gameplay = structuredClone(descriptor.gameplay);
const receiverPlacement = document.groups.find((group) => group.id === id).transform;
assert.equal(placement.rot_deg, 0);
assert.equal(receiverPlacement.rot_deg, 0);
const polygon = floor.polygon.map(([x, y]) => [
  x + placement.dx - receiverPlacement.dx,
  y + placement.dy - receiverPlacement.dy,
]);
const height = floor.height.map((z) => z + placement.dz - receiverPlacement.dz);
// Keep the opening clear of coincident clipping edges at subpixel precision.
// This margin is asset-local and follows the wall through every placement.
const scale = 1048576;
const ClipperLib = createRequire(new URL("../shared/package.json", import.meta.url))("clipper-lib");
const offset = new ClipperLib.ClipperOffset();
const path = polygon.map(([x, y]) => ({ X: Math.round(x * scale), Y: Math.round(y * scale) }));
if (!ClipperLib.Clipper.Orientation(path)) path.reverse();
offset.AddPath(path, ClipperLib.JoinType.jtMiter, ClipperLib.EndType.etClosedPolygon);
const expanded = [];
offset.Execute(expanded, scale / 1024);
assert.equal(expanded.length, 1);
const opening = expanded[0].map(({ X, Y }) => [X / scale, Y / scale]);
const plane = heightPlane(polygon.map(([x, y], i) => [x, y, height[i]]));
const changes = [];
for (const node of ["building-244--component-curtain-wall", "building-241"]) {
  const clearance = gameplay.movementClearances.find((c) => c.id === `${node}-clearance-13-0`);
  assert.ok(clearance);
  changes.push({ id: clearance.id, before: structuredClone(clearance) });
  clearance.polygon = structuredClone(opening);
  clearance.height = opening.map((point) => planeHeight(plane, point));
  clearance.holes = [];
  clearance.preserveMovementPrecision = true;
  changes.at(-1).after = structuredClone(clearance);
}
for (const [surface, corners] of [
  [gameplay.surfaces[0], [1, 2]],
  [
    gameplay.movementClearances.find(
      (c) => c.id === "building-244--component-curtain-wall-clearance-2-0",
    ),
    [2, 3],
  ],
  [gameplay.movementClearances.find((c) => c.id === "building-241-clearance-60-0"), [1, 2]],
]) {
  assert.ok(surface);
  for (const corner of corners) {
    const point = surface.polygon[corner];
    const before = [...point];
    const t =
      (surface.height[corner] - planeHeight(plane, point)) / (plane[0] ** 2 + plane[1] ** 2);
    const distance = Math.abs(t) * Math.hypot(plane[0], plane[1]);
    assert.ok(distance < 2, `Unreviewed contact correction ${distance}`);
    point[0] += t * plane[0];
    point[1] += t * plane[1];
    changes.push({ id: surface.id, corner, before, after: [...point], distance });
  }
  surface.preserveMovementPrecision = true;
}
gameplay.draft.issues.push(
  "Upper east stair opening candidate requires independent placement, contact mesh and rendered review before publication.",
);
validateAssetGameplay(gameplay, descriptor);
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8")).assets;
edits.push({
  asset: id,
  descriptorSha256: index.find((entry) => entry.id === id).descriptor_sha256,
  gameplay,
});
const output = await fs.mkdtemp("work/map-compile/york-east-upper-stair-contact-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(`${output}/review.json`, JSON.stringify({ stage, changes }));
for (const edit of edits) assets.get(edit.asset).gameplay = edit.gameplay;
const compiled = compileMap(document, [0, 0, ...document.size], assets, { bestEffort: true });
await fs.writeFile(`${output}/york.level.json`, JSON.stringify(compiled.descriptor));
await fs.writeFile(
  `${output}/diagnostics.json`,
  JSON.stringify({
    scope: "static-geometry-only-not-gameplay-parity",
    complete: true,
    results: [{ map: "york", file: "york.level.json", warnings: compiled.warnings }],
  }),
);
console.log(JSON.stringify({ output, changes: changes.length }));
