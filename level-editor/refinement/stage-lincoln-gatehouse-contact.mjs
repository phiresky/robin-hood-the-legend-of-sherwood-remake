import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { compileMap } from "../app/src/map-compile.ts";

// Author a candidate contact in the receiving asset, never in the compiler.
// Mesh and moved-neighbour checks must precede publication.
const [stage] = process.argv.slice(2);
assert.ok(stage, "Provide the reviewed gatehouse seam edits");
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8"));
assert.equal(edits.length, 1);
assert.equal(edits[0].asset, "lincoln-inner-gatehouse");
const document = await readStoredMap("library/scenes/lincoln.rhlos-map.json", "library");
const assets = await pinnedDescriptors("library", document.assetSources, document.sceneAssets);
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8")).assets;
assert.equal(
  edits[0].descriptorSha256,
  index.find((e) => e.id === edits[0].asset).descriptor_sha256,
);
const origin = (id) => {
  const group = document.groups.find((group) => group.id === id);
  assert.ok(group && group.transform.rot_deg === 0);
  assert.ok(
    document.objects
      .filter((o) => o.group === id)
      .every((o) => Object.entries(o.transform).every(([, value]) => value === 0)),
  );
  return group.transform;
};
const gatehouse = edits[0].gameplay;
const gateOrigin = origin(edits[0].asset);
const flight = gatehouse.surfaces.find((s) => s.id === "building-123-walk-0");
const plane = heightPlane(
  flight.polygon.map(([x, y], i) => [
    x + gateOrigin.dx,
    y + gateOrigin.dy,
    flight.height[i] + gateOrigin.dz,
  ]),
);
const id = "lincoln-castle-hill-inner-bailey-plateau";
const descriptor = assets.get(id);
const gameplay = structuredClone(descriptor.gameplay);
const placement = origin(id);
assert.equal(gameplay.collision, "parts");
assert.equal(descriptor.parts.length, 1);
assert.equal(gameplay.surfaces.length, 0);
assert.equal(gameplay.volumes, undefined);
assert.equal(gameplay.movementSolids, undefined);
assert.equal(gameplay.projectionReceivers.length, 1);
const part = descriptor.parts[0];
assert.equal(part.node, "building-062");
assert.ok(!part.collision && !part.default_hidden && !part.mission_profile);
assert.ok(!part.sight_join_caps && !part.sight_join_edges);
const shape = structuredClone(part.obstacle_local_game);
delete shape.projection_area;
delete shape.material_indices;
assert.ok(shape.points.every((p) => Math.abs(p.z_top - 220.001) < 1e-7));
const changes = [];
for (const [i, expected] of [
  [20, [2256.0005, 1499]],
  [21, [2306.9995, 1506]],
]) {
  const p = shape.points[i];
  const before = [p.x, p.y];
  const world = [p.x + placement.dx, p.y + placement.dy];
  assert.ok(Math.hypot(world[0] - expected[0], world[1] - expected[1]) < 1e-6);
  const t = (p.z_top + placement.dz - planeHeight(plane, world)) / (plane[0] ** 2 + plane[1] ** 2);
  const distance = Math.abs(t) * Math.hypot(plane[0], plane[1]);
  assert.ok(distance < 3, "Receiving contact exceeds explicit authoring bound");
  p.x += t * plane[0];
  p.y += t * plane[1];
  changes.push({ owner: id, vertex: i, before, after: [p.x, p.y], distance });
}
const volume = `${part.node}-authored-volume`;
gameplay.collision = "none";
gameplay.volumes = [{ id: volume, node: part.node, shape }];
gameplay.sightOrder = { [volume]: gameplay.sightOrder[part.node] };
for (const material of gameplay.materials)
  material.obstacles = material.obstacles.map((id) => (id === part.node ? volume : id));
gameplay.projectionReceivers = [];
gameplay.movementSolids = [volume];
gameplay.surfaces.push({
  id: `${part.node}-physical-walkway`,
  node: part.node,
  projectionVolume: volume,
  polygon: shape.points.map((p) => [p.x, p.y]),
  height: shape.points.map((p) => p.z_top),
  preserveMovementPrecision: true,
  preserveMovementBoundary: true,
  navigationRegion: `${part.node}-physical-walkway`,
});
// Existing ground exclusions still apply below the plateau. Their projected
// contours also describe obstructions on its receiving top; retain both layers.
gameplay.movementBlockers.push(
  ...gameplay.movementBlockers.map((blocker) => {
    assert.ok(blocker.height.every((height) => height === 0));
    return {
      ...structuredClone(blocker),
      id: `${blocker.id}-plateau-top`,
      polygon: blocker.polygon.map(([x, y]) => [x, y + 220.001]),
      holes: (blocker.holes ?? []).map((hole) => hole.map(([x, y]) => [x, y + 220.001])),
      height: blocker.height.map(() => 220.001),
    };
  }),
);
gameplay.draft.issues.push(
  "Gatehouse receiving contact and independently owned plateau floor are staged for mesh and moved-actor review; rendered integration remains unverified.",
);
validateAssetGameplay(gameplay, descriptor);
edits.push({
  asset: id,
  descriptorSha256: index.find((e) => e.id === id).descriptor_sha256,
  gameplay,
});
for (const edit of edits) assets.get(edit.asset).gameplay = edit.gameplay;
const output = await fs.mkdtemp("work/map-compile/lincoln-gatehouse-contact-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({ stage, changes, scope: "unpublished contact candidate" }),
);
console.log(JSON.stringify({ output, changes }));
const compiled = compileMap(document, document.exportBounds ?? [0, 0, ...document.size], assets, {
  bestEffort: true,
});
await fs.writeFile(`${output}/lincoln.level.json`, JSON.stringify(compiled.descriptor));
await fs.writeFile(
  `${output}/diagnostics.json`,
  JSON.stringify({
    complete: true,
    scope: "static-geometry-only-not-gameplay-parity",
    results: [{ file: "lincoln.level.json", map: "lincoln", warnings: compiled.warnings }],
  }),
);
console.log(
  JSON.stringify({
    complete: true,
    warnings: compiled.warnings.filter((w) => w.includes("building-123")),
  }),
);
