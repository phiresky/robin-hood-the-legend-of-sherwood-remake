import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

// Explicit asset authoring. Contact geometry remains local to its owning asset.
const [stage, offsetFlag, offsetValue] = process.argv.slice(2);
assert.ok(stage, "Provide reviewed gable-house climb seam edits");
assert.ok(
  process.argv.length <= 5 && (offsetFlag === undefined || offsetFlag === "--upper-seam-offset"),
);
const upperSeamOffset = offsetFlag ? Number(offsetValue) : 0;
assert.ok(
  Number.isFinite(upperSeamOffset) && Math.abs(upperSeamOffset) <= 2,
  "Upper seam offset requires a reviewed distance within two local game units",
);
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8"));
assert.equal(edits.length, 1);
const climbId = "york-central-lane-stone-gable-house";
const roofId = "york-central-lane-timber-lean-to";
assert.equal(edits[0].asset, climbId);
const scene = await readStoredMap("library/scenes/york.rhlos-map.json", "library");
const assets = await pinnedDescriptors("library", scene.assetSources, scene.sceneAssets);
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8")).assets;
assert.equal(
  edits[0].descriptorSha256,
  index.find((entry) => entry.id === climbId).descriptor_sha256,
);
const origin = (id) => {
  const group = scene.groups.find((group) => group.id === id);
  assert.ok(group && group.transform.rot_deg === 0);
  assert.ok(
    scene.objects
      .filter((object) => object.group === id)
      .every((object) => Object.values(object.transform).every((value) => value === 0)),
  );
  return [group.transform.dx, group.transform.dy, group.transform.dz];
};
const climbOrigin = origin(climbId),
  roofOrigin = origin(roofId);
const convert = (points, from, to) =>
  points.map((point) => point.map((value, axis) => value + from[axis] - to[axis]));
const climb = edits[0].gameplay;
const flight = climb.surfaces.find((surface) => surface.id === "building-146-walk-0");
const flightPoints = flight.polygon.map(([x, y], i) => [x, y, flight.height[i]]);
const localFlight = convert(flightPoints, climbOrigin, roofOrigin);
const flightPlane = heightPlane(localFlight);
const gameplay = structuredClone(assets.get(roofId).gameplay);
assert.ok(
  assets.get(roofId).parts.find((part) => part.node === "building-148").obstacle_local_game.solid,
);
const roof = gameplay.surfaces.find((surface) => surface.id === "building-148-walk-0");
const before = structuredClone(roof);
const footprint = roof.projectionMaterials.footprint;
assert.equal(footprint.length, 4);
const roofPlane = heightPlane(roof.projectionMaterials.planePoints);
const difference = flightPlane.map((value, i) => value - roofPlane[i]);
const length = Math.hypot(difference[0], difference[1]);
assert.ok(length > 0);
const changes = [];
if (upperSeamOffset !== 0) {
  const lift = climb.lifts.find((lift) => lift.surface === flight.id);
  assert.ok(lift && lift.doors.length === 2);
  const door = lift.doors[1];
  assert.ok(door.middle[2] > lift.doors[0].middle[2], "Expected the upper climb entrance");
  const before = structuredClone(door);
  const direction = [-difference[1] / length, difference[0] / length];
  for (const key of ["inside", "middle", "outside"]) {
    const point = door[key];
    point[0] += direction[0] * upperSeamOffset;
    point[1] += direction[1] * upperSeamOffset;
    const roofPoint = convert([point], climbOrigin, roofOrigin)[0];
    point[2] =
      planeHeight(key === "inside" ? flightPlane : roofPlane, roofPoint) +
      roofOrigin[2] -
      climbOrigin[2];
  }
  changes.push({
    kind: "upper-climb-seam-shift",
    distance: upperSeamOffset,
    direction,
    before,
    after: structuredClone(door),
  });
}
for (const i of [1, 2]) {
  const point = footprint[i];
  const t = -planeHeight(difference, point) / length ** 2;
  const distance = Math.abs(t) * length;
  assert.ok(distance < 2.2, `Roof contact needs separate review: ${distance}`);
  const previous = [...point];
  point[0] += t * difference[0];
  point[1] += t * difference[1];
  point[2] = planeHeight(roofPlane, point);
  changes.push({ vertex: i, distance, before: previous, after: [...point] });
}
roof.polygon = footprint.map(([x, y]) => [x, y]);
roof.height = footprint.map((point) => planeHeight(roofPlane, point));
roof.preserveMovementPrecision = true;
roof.preserveMovementBoundary = true;
roof.navigationRegion = roof.id;
// The climb occupies a separate floor through the roof edge. Keep its footprint
// out of ordinary roof walking; the explicit entrance supplies the handoff.
const cutoutId = "building-148-climb-cutout";
assert.ok(!gameplay.movementBlockers?.some((blocker) => blocker.id === cutoutId));
gameplay.movementBlockers ??= [];
gameplay.movementBlockers.push({
  id: cutoutId,
  node: roof.node,
  polygon: localFlight.map(([x, y]) => [x, y]),
  height: localFlight.map((point) => planeHeight(roofPlane, point)),
  preserveMovementPrecision: true,
});
// Keep the roof slab's movement volume aligned with its authored contact.
// The visual/query part retains its independent geometry.
const volumeId = "building-148-contact-slab";
assert.ok(!gameplay.volumes?.some((volume) => volume.id === volumeId));
const originalSlab = assets
  .get(roofId)
  .parts.find((part) => part.node === roof.node).obstacle_local_game;
const {
  projection_area: _projectionArea,
  material_indices: _materialIndices,
  ...shape
} = originalSlab;
assert.ok(originalSlab.points.every((point) => point.z_bottom === originalSlab.points[0].z_bottom));
gameplay.volumes ??= [];
gameplay.volumes.push({
  id: volumeId,
  node: roof.node,
  shape: {
    ...shape,
    opaque: false,
    mouse: false,
    show_shadow_polygon: false,
    points: footprint.map(([x, y], i) => ({
      x,
      y,
      z_bottom: originalSlab.points[0].z_bottom,
      z_top: roof.height[i],
    })),
  },
});
gameplay.movementSolids = [...new Set([...(gameplay.movementSolids ?? []), volumeId])];
changes.push({ kind: "roof-contact-slab", before: originalSlab, after: gameplay.volumes.at(-1) });
const clearance = (target, id, node, points) => {
  assert.ok(!target.movementClearances?.some((entry) => entry.id === id));
  target.movementClearances ??= [];
  target.movementClearances.push({
    id,
    node,
    polygon: points.map(([x, y]) => [x, y]),
    height: points.map((point) => point[2]),
    preserveMovementPrecision: true,
  });
};
clearance(
  climb,
  "building-146-lean-to-roof-opening",
  flight.node,
  convert(
    roof.polygon.map(([x, y], i) => [x, y, roof.height[i]]),
    roofOrigin,
    climbOrigin,
  ),
);
gameplay.draft.issues.push(
  "Lean-to roof contact is an unpublished authoring candidate; mesh and placed actor review required.",
);
climb.draft.issues.push(
  "Gable climb candidate retains visible mesh coverage gaps; rendered integration remains unverified.",
);
validateAssetGameplay(climb, assets.get(climbId));
validateAssetGameplay(gameplay, assets.get(roofId));
edits.push({
  asset: roofId,
  descriptorSha256: index.find((entry) => entry.id === roofId).descriptor_sha256,
  gameplay,
});
const output = await fs.mkdtemp("work/map-compile/york-gable-roof-contact-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({ stage, before, after: roof, changes }),
);
console.log(JSON.stringify({ output, changes }));
