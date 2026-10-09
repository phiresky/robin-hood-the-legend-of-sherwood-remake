import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createJumpClearance } from "../shared/src/jump-clearance.ts";

const directory = process.argv[2];
assert.ok(directory, "Supply a diagnostics directory recorded with ROBIN_TRACE_JUMP=1");
const report = JSON.parse(
  await fs.readFile(`${directory}/${process.argv[3] ?? "actor-jump-vertical-report.json"}`, "utf8"),
);
assert.equal(report.complete, true);
let samples = 0;
const checkedPositions = new Map();
for (const result of report.results) {
  const descriptor = JSON.parse(await fs.readFile(`${directory}/${result.file}`, "utf8"));
  const pair = descriptor.asset_geometry.jump_line_pairs[Math.floor(result.line / 2)];
  const edges = [pair.line1, pair.line2].map((line, index) => ({
    zone: `${index}`,
    a: [line.point_a[0], line.point_a[1] + line.point_a[2], line.point_a[2]],
    b: [line.point_b[0], line.point_b[1] + line.point_b[2], line.point_b[2]],
  }));
  const parameter = result.line % 2 ? 1 - result.t : result.t;
  const motionAreas = new Map();
  let sector = 0;
  for (const areas of descriptor.asset_geometry.motion_data.layers)
    for (const area of areas) {
      motionAreas.set(sector, area.polygon.points);
      sector += 1 + area.obstacles.length;
    }
  const receivingSurfaces = new Map(
    [pair.line2, pair.line1].map((opposite, index) => {
      const zone = descriptor.asset_geometry.jump_zones[opposite.jump_zone_index];
      return [
        `${index}`,
        {
          topology: { sector: zone.sector, layer: zone.layer },
          motionPolygon: motionAreas.get(zone.sector),
        },
      ];
    }),
  );
  assert.ok(result.arrival.trajectory?.length, "Native trajectory recording is required");
  for (const frame of result.arrival.trajectory) {
    const [x, y, z] = frame.position;
    // A tiny physical obstacle at every observed foot position must obstruct
    // that part of the exported span. Check the full path, including transitions.
    const obstacle = {
      points: [
        [x - 0.02, y - 0.02],
        [x + 0.02, y - 0.02],
        [x + 0.02, y + 0.02],
        [x - 0.02, y + 0.02],
      ].map(([x, y]) => ({ x, y, z_bottom: z - 0.02, z_top: z + 0.02 })),
      solid: true,
      opaque: false,
      mouse: false,
      show_shadow_polygon: false,
      default_material: 0,
      projection_area: null,
      material_indices: [],
    };
    const receivingPlanes = descriptor.asset_geometry.sight_obstacles
      .filter((shape) => shape.projection_area)
      .map((shape) => ({ ...shape, solid: false }));
    const key = `${result.file}:${Math.floor(result.line / 2)}:${frame.position.join(",")}`;
    let blocked = checkedPositions.get(key);
    if (!blocked) {
      blocked = createJumpClearance([...receivingPlanes, obstacle], receivingSurfaces)(
        edges,
        pair.jump_long,
      );
      checkedPositions.set(key, blocked);
    }
    assert.ok(
      blocked.some(([a, b]) => parameter >= a - 0.00001 && parameter <= b + 0.00001),
      `${result.file} line ${result.line} t=${result.t}: uncovered ${JSON.stringify(frame)}`,
    );
    samples++;
  }
}
assert.ok(samples > 0);
console.log(
  JSON.stringify({
    complete: true,
    cases: report.results.length,
    samples,
    uniquePositions: checkedPositions.size,
  }),
);
