// Inspect representable receiving points without changing assets or runtime data.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { pointInGameplayPolygon } from "../shared/src/navigation-anchor.ts";

const [file, pairArgument = "0"] = process.argv.slice(2);
assert.ok(file, "Supply an exported descriptor and optional jump pair index");
const pairIndex = Number(pairArgument);
assert.ok(Number.isInteger(pairIndex) && pairIndex >= 0);
const geometry = JSON.parse(await fs.readFile(file)).asset_geometry;
const pair = geometry.jump_line_pairs[pairIndex];
assert.ok(pair, "Requested pair must exist");
const lines = [pair.line1, pair.line2];
const sides = lines.map((line, side) => {
  const zone = geometry.jump_zones[lines[1 - side].jump_zone_index];
  const midpoint = line.point_a.map((value, axis) => (value + line.point_b[axis]) / 2);
  const faces = [];
  for (const [index, receiver] of geometry.sight_obstacles.entries()) {
    if (
      !Array.isArray(receiver.projection_area) ||
      receiver.projection_area[0] !== zone.sector ||
      receiver.projection_area[1] !== zone.layer
    )
      continue;
    const plane = heightPlane(
      receiver.projection_plane ?? receiver.points.slice(0, 3).map((p) => [p.x, p.y, p.z_top]),
    );
    const determinant = 1 - plane[1];
    if (Math.abs(determinant) < 1e-10) {
      faces.push({ index, plane, determinant, error: "Receiving plane projects to a line" });
      continue;
    }
    const boundary = receiver.points.map((p) => [p.x, p.y]);
    const screenBoundary = boundary.map(([x, y]) => [x, y - planeHeight(plane, [x, y])]);
    const fromScreen = ([x, y]) => {
      const worldY = (y + plane[0] * x + plane[2]) / determinant;
      return [x, worldY, planeHeight(plane, [x, worldY])];
    };
    const low = [0, 1].map((axis) =>
      Math.max(
        Math.ceil(Math.min(...screenBoundary.map((p) => p[axis]))),
        Math.floor(midpoint[axis] - 32),
      ),
    );
    const high = [0, 1].map((axis) =>
      Math.min(
        Math.floor(Math.max(...screenBoundary.map((p) => p[axis]))),
        Math.ceil(midpoint[axis] + 32),
      ),
    );
    const integerPoints = [];
    for (let x = low[0]; x <= high[0]; x++)
      for (let y = low[1]; y <= high[1]; y++) {
        const world = fromScreen([x, y]);
        if (pointInGameplayPolygon(world.slice(0, 2), boundary, true))
          integerPoints.push({ screen: [x, y], world });
      }
    const probes = [0, 0.25, 0.5, 0.75, 1].map((t) => {
      const screen = line.point_a
        .slice(0, 2)
        .map((value, axis) => value + t * (line.point_b[axis] - value));
      const world = fromScreen(screen);
      return {
        t,
        screen,
        world,
        supported: pointInGameplayPolygon(world.slice(0, 2), boundary, true),
      };
    });
    faces.push({ index, plane, determinant, probes, integerPoints });
  }
  return { side, line, zone, faces };
});
const output = await fs.mkdtemp("work/map-compile/jump-integer-support-");
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({
    scope: "receiver-representation-not-actor-clearance-or-traversal",
    file,
    pairIndex,
    sides,
  }),
);
console.log(output);
console.log(
  JSON.stringify(
    sides.map(({ side, faces }) => ({
      side,
      faces: faces.map(({ index, determinant, integerPoints, probes, error }) => ({
        index,
        determinant,
        integerPoints: integerPoints?.length,
        supportedEdgeProbes: probes?.filter((probe) => probe.supported).length,
        error,
      })),
    })),
  ),
);
