import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { compileMap } from "../app/src/map-compile.ts";

// Explicit unpublished authoring candidate. The structural flight is narrower
// than its recovered navigation envelope; mesh and moved-contact review remain
// required before applying these asset-local definitions to the library.
const [stage] = process.argv.slice(2);
assert.ok(stage);
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8"));
const document = await readStoredMap("library/scenes/york.rhlos-map.json", "library");
const assets = await pinnedDescriptors("library", document.assetSources, document.sceneAssets);
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8")).assets;
const id = "york-riverbank-wooden-landing-steps";
assert.ok(!edits.some((edit) => edit.asset === id));
const descriptor = assets.get(id);
const gameplay = structuredClone(descriptor.gameplay);
const lift = gameplay.lifts[0];
const floor = gameplay.surfaces.find((surface) => surface.id === lift.surface);
const part = descriptor.parts.find((part) => part.node === lift.node);
const points = structuredClone(part.obstacle_local_game.points);
assert.equal(points.length, 4);
const plane = heightPlane(points.slice(0, 3).map((p) => [p.x, p.y, p.z_top]));
for (const point of points) {
  const t =
    (point.z_top - planeHeight(plane, [point.x, point.y])) / (plane[0] ** 2 + plane[1] ** 2);
  assert.ok(Math.abs(t) * Math.hypot(plane[0], plane[1]) < 0.001);
  point.x += t * plane[0];
  point.y += t * plane[1];
}
floor.polygon = points.map((p) => [p.x, p.y]);
floor.height = points.map((p) => p.z_top);
floor.preserveMovementPrecision = true;
floor.projectionMaterials.planePoints = points.slice(0, 3).map((p) => [p.x, p.y, p.z_top]);
floor.projectionMaterials.footprint = points.map((p) => [p.x, p.y, p.z_top]);
const direction = [plane[0], plane[1]].map((v) => v / Math.hypot(plane[0], plane[1]));
for (const [i, door] of lift.doors.entries()) {
  const edge = i ? points.slice(0, 2) : points.slice(2);
  assert.ok(Math.abs(edge[0].z_top - edge[1].z_top) < 1e-7);
  const middle = ["x", "y"].map((key) => (edge[0][key] + edge[1][key]) / 2);
  const sign = i ? -1 : 1;
  const inside = middle.map((v, axis) => v + sign * direction[axis] * 8);
  door.middle = [...middle, edge[0].z_top];
  door.inside = [...inside, planeHeight(plane, inside)];
  door.outside = [...middle.map((v, axis) => v - sign * direction[axis] * 12), edge[0].z_top];
}
const flightClearances = gameplay.movementClearances.filter(
  (clearance) =>
    clearance.node === floor.node &&
    Array.isArray(clearance.height) &&
    Math.max(...clearance.height) - Math.min(...clearance.height) > 1,
);
assert.equal(flightClearances.length, 1);
gameplay.movementClearances = gameplay.movementClearances.filter(
  (clearance) => !flightClearances.includes(clearance),
);
gameplay.movementClearances.push({
  id: `${floor.id}-flight-clearance`,
  node: floor.node,
  polygon: structuredClone(floor.polygon),
  height: [...floor.height],
  holes: [],
});
gameplay.draft.issues.push(
  "Narrowed structural stair floor candidate requires visible mesh and both independent landing contact reviews.",
);
validateAssetGameplay(gameplay, descriptor);
edits.unshift({
  asset: id,
  descriptorSha256: index.find((entry) => entry.id === id).descriptor_sha256,
  gameplay,
});
const stairPlacement = document.groups.find((group) => group.id === id).transform;
assert.equal(stairPlacement.rot_deg, 0);
for (const [neighbour, node, corners] of [
  [
    "york-southeast-riverside-raised-terrace",
    "building-093",
    [
      [5, 1],
      [6, 0],
    ],
  ],
  [
    "york-east-riverside-wooden-walkway",
    "building-024",
    [
      [0, 2],
      [8, 3],
    ],
  ],
]) {
  const edit = edits.find((edit) => edit.asset === neighbour);
  assert.ok(edit);
  const data = edit.gameplay;
  const placement = document.groups.find((group) => group.id === neighbour).transform;
  assert.equal(placement.rot_deg, 0);
  const surface = data.surfaces[0];
  let volume = data.volumes?.find((volume) => volume.id === surface.projectionVolume);
  if (!volume) {
    const source = assets
      .get(neighbour)
      .parts.find((part) => part.node === node).obstacle_local_game;
    const volumeId = `${node}-reviewed-receiver`;
    volume = { id: volumeId, node, shape: structuredClone(source) };
    delete volume.shape.projection_area;
    delete volume.shape.material_indices;
    data.volumes = [volume];
    data.collision = "none";
    data.movementSolids = [volumeId];
    data.sightOrder = { [volumeId]: data.sightOrder[node] };
    for (const material of data.materials ?? [])
      material.obstacles = material.obstacles.map((id) => (id === node ? volumeId : id));
    surface.projectionVolume = volumeId;
  }
  for (const [corner, stairCorner] of corners) {
    const target = volume.shape.points[corner];
    const point = points[stairCorner];
    const x = point.x + stairPlacement.dx - placement.dx;
    const y = point.y + stairPlacement.dy - placement.dy;
    assert.ok(
      Math.hypot(target.x - x, target.y - y) < 0.6,
      "Unexpected neighbouring contact correction",
    );
    target.x = x;
    target.y = y;
  }
  surface.polygon = volume.shape.points.map((point) => [point.x, point.y]);
  surface.height = volume.shape.points.map((point) => point.z_top);
  data.draft.issues.push(
    "Wooden stair contact corners corrected within 0.6 game units; independent moved contact and mesh checks remain required.",
  );
  validateAssetGameplay(data, assets.get(neighbour));
}
for (const edit of edits) assets.get(edit.asset).gameplay = edit.gameplay;
const output = await fs.mkdtemp("work/map-compile/york-wooden-stair-floor-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
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
console.log(
  JSON.stringify({
    output,
    warnings: compiled.warnings.filter((warning) =>
      /traversal omitted|wooden-landing-steps.*unavailable/.test(warning),
    ),
  }),
);
