// Keep a plateau's top exclusion inside its own authored walking footprint.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createHash } from "node:crypto";
import { fixedPolygonBoolean } from "../shared/src/fixed-polygon-boolean.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { signedPolygonArea } from "../shared/src/geometry.ts";

const [stage, mode] = process.argv.slice(2);
assert.ok(stage, "Supply the staged drawbridge floors");
assert.ok(mode === undefined || mode === "--trim-floor");
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`));
const root = "library/3d-assets/lincoln/lincoln-castle-hill-inner-bailey-plateau";
const bytes = await fs.readFile(`${root}/asset.json`);
const descriptor = JSON.parse(bytes);
assert.ok(!edits.some((edit) => edit.asset === descriptor.id));
const gameplay = structuredClone(descriptor.gameplay);
const surface = gameplay.surfaces.find((surface) => surface.id === "building-062-physical-walkway");
assert.ok(surface?.preserveMovementBoundary);
assert.ok(surface.height.every((z) => Math.abs(z - 220.001) < 1e-6));
const footprint = [[surface.polygon, ...(surface.holes ?? [])]];
const changes = [];
gameplay.movementBlockers = gameplay.movementBlockers.flatMap((blocker) => {
  if (!blocker.id.endsWith("-plateau-top")) return [blocker];
  assert.equal(blocker.node, surface.node);
  assert.ok(blocker.height.every((z) => Math.abs(z - 220.001) < 1e-6));
  const clipped = fixedPolygonBoolean(
    "intersection",
    [[blocker.polygon, ...blocker.holes]],
    [footprint],
  );
  const replacements = clipped.map(([outer, ...holes], index) => ({
    ...blocker,
    id: `${blocker.id}-owned-${index}`,
    polygon: outer.slice(0, -1),
    holes: holes.map((hole) => hole.slice(0, -1)),
    height: outer.slice(0, -1).map(() => 220.001),
  }));
  const area = (polygons) =>
    polygons.reduce(
      (sum, [outer, ...holes]) =>
        sum +
        Math.abs(signedPolygonArea(outer)) -
        holes.reduce((n, hole) => n + Math.abs(signedPolygonArea(hole)), 0),
      0,
    );
  changes.push({
    id: blocker.id,
    beforeArea: area([[blocker.polygon, ...blocker.holes]]),
    afterArea: area(clipped),
    pieces: replacements.length,
  });
  return replacements;
});
assert.equal(changes.length, 7);
assert.ok(changes.some((change) => change.beforeArea > change.afterArea + 1));
if (mode === "--trim-floor") {
  const top = gameplay.movementBlockers.filter((blocker) =>
    blocker.id.includes("-plateau-top-owned-"),
  );
  const remaining = fixedPolygonBoolean(
    "difference",
    footprint,
    top.map((blocker) => [[blocker.polygon, ...blocker.holes]]),
  );
  assert.ok(remaining.length, "Plateau floor disappeared");
  const replacement = remaining.map(([outer, ...holes], index) => {
    const result = {
      ...surface,
      id: index === 0 ? surface.id : `${surface.id}-piece-${index}`,
      polygon: outer.slice(0, -1),
      height: outer.slice(0, -1).map(() => 220.001),
      holes: holes.map((hole) => hole.slice(0, -1)),
    };
    if (remaining.length > 1) delete result.preserveMovementBoundary;
    return result;
  });
  gameplay.surfaces = gameplay.surfaces.flatMap((entry) =>
    entry === surface ? replacement : [entry],
  );
  gameplay.movementBlockers = gameplay.movementBlockers.filter((blocker) => !top.includes(blocker));
}
gameplay.draft.issues.push(
  "Unpublished plateau exclusion ownership candidate: verify unchanged internal clearance and connected drawbridge traversal before publication.",
);
validateAssetGameplay(gameplay, descriptor);
const sha = (data) => createHash("sha256").update(data).digest("hex");
edits.push({
  asset: descriptor.id,
  descriptorSha256: sha(bytes),
  modelSha256: sha(await fs.readFile(`${root}/model.glb`)),
  gameplay,
});
const output = await fs.mkdtemp("work/needs-review/map-compile/lincoln-plateau-exclusion-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(
  `${output}/ownership-review.json`,
  JSON.stringify({ stage, mode, asset: descriptor.id, changes, published: false }),
);
console.log(output);
