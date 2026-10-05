// Compare necessary actor-footprint containment before and after stair projection.
// Run from level-editor with a gameplay JSON and an output report path.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { heightPlane } from "../shared/src/gameplay-plane.ts";
import { gameTransformMatrix } from "../shared/src/level3d.ts";
import { gameToScene } from "../shared/src/scene.ts";
import { applyAffineMatrix, sceneToGame, signedPolygonArea } from "../shared/src/geometry.ts";

const require = createRequire(new URL("../pipeline/package.json", import.meta.url));
const clipping = require("polygon-clipping");
const [input, output] = process.argv.slice(2);
if (!input || !output)
  throw new Error("Usage: audit-stair-footprints.mjs gameplay.json report.json");
const gameplay = JSON.parse(await fs.readFile(input, "utf8"));
const camera = { kind: "oblique-orthographic", elevation_deg: 35 };
const corners = [
  [-6, -3],
  [-6, 3],
  [6, -3],
  [6, 3],
];
const area = (polygons) =>
  polygons.reduce(
    (total, polygon) =>
      total +
      polygon.reduce((sum, ring, i) => sum + (i ? -1 : 1) * Math.abs(signedPolygonArea(ring)), 0),
    0,
  );
const centers = (polygon, offsets) =>
  area(
    clipping.intersection(
      ...offsets.map(([dx, dy]) => [polygon.map(([x, y]) => [x - dx, y - dy])]),
    ),
  );
const results = [];
const edgeOn = [];
for (const lift of gameplay.lifts ?? []) {
  if (lift.type !== 1) continue;
  const surface = gameplay.surfaces.find((entry) => entry.id === lift.surface);
  if (!surface) throw new Error(`Missing lift surface ${lift.surface}`);
  const placed = (rotation) => {
    const matrix = gameTransformMatrix(camera, { dx: 0, dy: 0, dz: 0, rot_deg: rotation }, [0, 0]);
    return surface.polygon.map(([x, y], i) =>
      sceneToGame(
        camera,
        applyAffineMatrix(
          matrix,
          gameToScene(
            camera,
            x,
            y,
            typeof surface.height === "number" ? surface.height : surface.height[i],
          ),
        ),
      ),
    );
  };
  const initialPlane = heightPlane(placed(0), false);
  const sine = initialPlane[0] / Math.sin((camera.elevation_deg * Math.PI) / 180);
  const cosine = initialPlane[1];
  const amplitude = Math.hypot(sine, cosine);
  if (amplitude >= 1) {
    const phase = Math.atan2(sine, cosine);
    const offset = Math.acos(1 / amplitude);
    for (const angle of [phase - offset, phase + offset]) {
      const rotation = ((angle * 180) / Math.PI + 360) % 360;
      const points = placed(rotation);
      const projectedArea = Math.abs(signedPolygonArea(points.map(([x, y, z]) => [x, y - z])));
      const physicalArea = Math.abs(signedPolygonArea(points.map(([x, y]) => [x, y])));
      assert.ok(projectedArea < 1e-6 * physicalArea, `Expected edge-on surface ${surface.id}`);
      edgeOn.push({ surface: surface.id, rotation, physicalArea, projectedArea });
    }
  }
  for (const rotation of [0, 37, 90, 180, 270]) {
    const points = placed(rotation);
    const plane = heightPlane(points, false);
    const physical = points.map(([x, y]) => [x, y]);
    const projected = points.map(([x, y, z]) => [x, y - z]);
    const footprint = corners.map(([dx, dy]) => [dx, dy - plane[0] * dx - plane[1] * dy]);
    const halfX = Math.max(...footprint.map(([x]) => Math.abs(x)));
    const halfY = Math.max(...footprint.map(([, y]) => Math.abs(y)));
    const bounds = [
      [-halfX, -halfY],
      [-halfX, halfY],
      [halfX, -halfY],
      [halfX, halfY],
    ];
    const physicalCenterArea = centers(physical, corners);
    const projectedFootprintCenterArea = centers(projected, footprint);
    const planeResidual = Math.max(
      ...points.map(([x, y, z]) => Math.abs(z - plane[0] * x - plane[1] * y - plane[2])),
    );
    assert.ok(planeResidual < 1e-6, `Nonplanar stair ${surface.id}: ${planeResidual}`);
    // An invertible affine projection must scale the feasible center area by
    // its determinant when the footprint is transformed with the floor.
    const expectedArea = physicalCenterArea * Math.abs(1 - plane[1]);
    assert.ok(
      Math.abs(projectedFootprintCenterArea - expectedArea) < 1e-5 * Math.max(1, expectedArea),
      `Projected containment disagrees with physical containment for ${surface.id}/${rotation}`,
    );
    results.push({
      surface: surface.id,
      rotation,
      heightPlane: plane,
      projectionDeterminant: 1 - plane[1],
      physicalCenterArea,
      currentScreenBoxCenterArea: centers(projected, corners),
      projectedFootprintCenterArea,
      projectedBoundingBoxCenterArea: centers(projected, bounds),
    });
  }
}
await fs.writeFile(
  output,
  JSON.stringify(
    {
      input,
      footBox: [12, 6],
      scope:
        "Necessary corner-containment only; ignores holes, solids, door approaches and route connectivity. Not a traversal certification.",
      results,
      edgeOn,
    },
    null,
    2,
  ),
);
console.log(JSON.stringify({ output, placements: results.length, edgeOn }));
