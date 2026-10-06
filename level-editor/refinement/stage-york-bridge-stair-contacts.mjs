import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { compileMap } from "../app/src/map-compile.ts";

const [stage] = process.argv.slice(2);
assert.ok(stage);
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8"));
const document = await readStoredMap("library/scenes/york.rhlos-map.json", "library");
const assets = await pinnedDescriptors("library", document.assetSources, document.sceneAssets);
const bridgeId = "york-stone-river-bridge-and-approach-stairs";
const bridge = edits.find((edit) => edit.asset === bridgeId).gameplay;
const terraceId = "york-east-bridge-raised-terrace";
const terrace = edits.find((edit) => edit.asset === terraceId).gameplay;
const origin = (id) => {
  const transform = document.groups.find((group) => group.id === id).transform;
  assert.equal(transform.rot_deg, 0);
  return transform;
};
const bridgeOrigin = origin(bridgeId);
const changes = [];
function align(point, height, plane, placement, owner) {
  const world = [point[0] + placement.dx, point[1] + placement.dy];
  const t = (height + placement.dz - planeHeight(plane, world)) / (plane[0] ** 2 + plane[1] ** 2);
  const distance = Math.abs(t) * Math.hypot(plane[0], plane[1]);
  assert.ok(distance < 2, "Bridge receiving contact exceeds reviewed authoring bound");
  const after = point.map((value, axis) => value + t * plane[axis]);
  changes.push({ owner, before: point, after, distance });
  return after;
}
function flightPlane(id) {
  const surface = bridge.surfaces.find((surface) => surface.id === id);
  assert.ok(surface);
  return heightPlane(
    surface.polygon.map(([x, y], i) => [
      x + bridgeOrigin.dx,
      y + bridgeOrigin.dy,
      surface.height[i] + bridgeOrigin.dz,
    ]),
  );
}
const upper = terrace.surfaces.find((surface) => surface.id === "building-092-walk-0");
for (const corner of [
  [-207, 79.6891470426196],
  [-256, 96.68914704261961],
]) {
  const index = upper.polygon.findIndex(
    (point) => Math.hypot(...point.map((v, i) => v - corner[i])) < 1e-6,
  );
  assert.ok(index >= 0, "Reviewed upper bridge contact changed");
  upper.polygon[index] = align(
    upper.polygon[index],
    upper.height[index],
    flightPlane("building-094-walk-0"),
    origin(terraceId),
    terraceId,
  );
}
upper.preserveMovementPrecision = true;

// Give the bridge's lower deck its own walkable floor. Keep every other solid
// unchanged while replacing the receiving contour with an authored volume.
assert.equal(bridge.collision, "parts");
assert.equal(bridge.volumes, undefined);
assert.equal(bridge.materials, undefined);
assert.equal(bridge.masks, undefined);
assert.equal(bridge.movementTransitions, undefined);
assert.equal(bridge.movementSolids, undefined);
assert.ok(Array.isArray(bridge.movementBlockers), "Keep explicit movement-contour ownership");
assert.ok(bridge.surfaces.every((surface) => surface.projectionVolume === undefined));
assert.equal(bridge.projectionReceivers.length, 1);
assert.equal(bridge.projectionReceivers[0].volume, "building-095");
bridge.volumes = assets
  .get(bridgeId)
  .parts.filter((part) => part.obstacle_local_game)
  .map((part) => {
    assert.ok(
      !part.collision && !part.mission_profile && !part.sight_join_caps && !part.sight_join_edges,
    );
    assert.ok(!part.default_hidden, "Do not turn a hidden state into an unconditional volume");
    const shape = structuredClone(part.obstacle_local_game);
    delete shape.projection_area;
    delete shape.material_indices;
    return { id: `${part.node}-authored-volume`, node: part.node, shape };
  });
bridge.collision = "none";
bridge.sightOrder = Object.fromEntries(
  Object.entries(bridge.sightOrder).map(([id, order]) => [`${id}-authored-volume`, order]),
);
bridge.projectionReceivers = [];
const receiver = bridge.volumes.find((volume) => volume.node === "building-095");
const plane = flightPlane("building-097-walk-0");
assert.equal(receiver.shape.points.length, 5);
for (const i of [0, 4]) {
  const point = receiver.shape.points[i];
  [point.x, point.y] = align([point.x, point.y], point.z_top, plane, bridgeOrigin, receiver.id);
}
bridge.movementSolids = [receiver.id];
bridge.surfaces.push({
  id: "building-095-physical-walkway",
  node: receiver.node,
  projectionVolume: receiver.id,
  polygon: receiver.shape.points.map((point) => [point.x, point.y]),
  height: receiver.shape.points.map((point) => point.z_top),
  holes: [],
  preserveMovementPrecision: true,
  preserveMovementBoundary: true,
  navigationRegion: "building-095-physical-walkway",
});
for (const gameplay of [bridge, terrace])
  gameplay.draft.issues.push(
    "Bridge stair receiving contours are staged for mesh and moved-actor review; rendered integration remains unverified.",
  );
for (const edit of edits) {
  validateAssetGameplay(edit.gameplay, assets.get(edit.asset));
  assets.get(edit.asset).gameplay = edit.gameplay;
}
const output = await fs.mkdtemp("work/map-compile/york-bridge-stair-contacts-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(`${output}/review.json`, JSON.stringify({ stage, changes }));
console.log(JSON.stringify({ output, changes }));
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
console.log(JSON.stringify({ output, complete: true }));
