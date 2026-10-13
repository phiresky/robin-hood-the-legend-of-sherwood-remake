// Review planar terrain exclusions against independently placed structural parts.
// Reports geometric overlap only; never rewrites collision or publishes assets.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { transformedObstacle } from "../shared/src/level3d.ts";
import { heightPlane } from "../shared/src/gameplay-plane.ts";
import { movementVolumeHeightSlice } from "../shared/src/movement-volume-height-slice.ts";
import { fixedPolygonBoolean } from "../shared/src/fixed-polygon-boolean.ts";

const [map, asset] = process.argv.slice(2);
assert.ok(map && asset, "Supply saved map and terrain asset ID");
const scene = await readStoredMap(`library/scenes/${map}.rhlos-map.json`, "library");
const assets = await pinnedDescriptors("library", scene.assetSources, scene.sceneAssets);
const terrain = assets.get(asset);
assert.ok(terrain?.gameplay);
const groups = scene.groups.filter((group) => group.id === asset);
assert.equal(groups.length, 1, "Audit requires one named terrain placement");
const transform = groups[0].transform;
assert.equal(transform.rot_deg, 0, "This snapshot audit requires unrotated axes");
const area = (polygons) =>
  polygons.reduce(
    (sum, polygon) =>
      sum +
      polygon.reduce(
        (total, ring, index) =>
          total +
          ((index === 0 ? 1 : -1) *
            Math.abs(
              ring.reduce((value, [x, y], i) => {
                const next = ring[(i + 1) % ring.length];
                return value + x * next[1] - y * next[0];
              }, 0),
            )) /
            2,
        0,
      ),
    0,
  );
const rows = [];
for (const blocker of terrain.gameplay.movementBlockers ?? []) {
  const heights =
    typeof blocker.height === "number" ? blocker.polygon.map(() => blocker.height) : blocker.height;
  const ring = blocker.polygon.map(([x, y]) => [x + transform.dx, y + transform.dy]);
  const holes = (blocker.holes ?? []).map((hole) =>
    hole.map(([x, y]) => [x + transform.dx, y + transform.dy]),
  );
  const polygon = [ring, ...holes];
  const plane = heightPlane(ring.map(([x, y], i) => [x, y, heights[i] + transform.dz]));
  let uncovered = [polygon];
  const contributors = [];
  for (const object of scene.objects) {
    if (object.group === asset || !object.obstacle?.solid) continue;
    const shape = transformedObstacle(scene, object);
    const footprint = shape.points.map((p) => [p.x, p.y]);
    const bottom = heightPlane(
      shape.points.map((p) => [p.x, p.y, p.z_bottom]),
      false,
    );
    const top = heightPlane(
      shape.points.map((p) => [p.x, p.y, p.z_top]),
      false,
    );
    const slice = movementVolumeHeightSlice(footprint, plane, bottom, top);
    if (slice.length < 3) continue;
    const cut = fixedPolygonBoolean("intersection", [footprint], [[slice]]);
    const overlap = fixedPolygonBoolean("intersection", polygon, cut);
    const overlapArea = area(overlap);
    if (overlapArea < 1e-7) continue;
    contributors.push({ placement: object.group, object: object.id, area: overlapArea });
    uncovered = fixedPolygonBoolean("difference", uncovered, cut);
  }
  rows.push({
    blocker: blocker.id,
    plane,
    area: area([polygon]),
    uncoveredArea: area(uncovered),
    contributors,
    polygon,
    uncovered,
  });
}
const output = await fs.mkdtemp("work/needs-review/map-compile/terrain-exclusion-ownership-");
await fs.writeFile(
  `${output}/report.json`,
  JSON.stringify({
    scope: "structural-overlap-review-not-gameplay-authorization",
    map,
    asset,
    assetSources: scene.assetSources,
    limitations: [
      "Solid parts include unselected, mission-profile and changing geometry; overlap does not authorize permanent movement collision.",
      "Planar blockers, authored clearances and mesh details require separate review.",
      "No overlap is not proof that a terrain exclusion is invalid.",
    ],
    rows,
  }),
);
console.log(
  JSON.stringify({
    output,
    blockers: rows.length,
    area: rows.reduce((sum, row) => sum + row.area, 0),
    uncoveredArea: rows.reduce((sum, row) => sum + row.uncoveredArea, 0),
    contributors: new Set(rows.flatMap((row) => row.contributors.map((c) => c.object))).size,
  }),
);
