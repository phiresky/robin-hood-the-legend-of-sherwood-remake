// Diagnostic only: verify shared stair approach preparation before compiler integration.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { compileLiftApproaches } from "../shared/src/compile-lift-approaches.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { pointInGameplayPolygon } from "../shared/src/navigation-anchor.ts";

const [source] = process.argv.slice(2);
assert.ok(source);
const manifest = JSON.parse(await fs.readFile(`${source}/diagnostics.json`));
assert.equal(manifest.complete, true);
const output = await fs.mkdtemp("work/needs-review/map-compile/hall-ordinary-approach-diagnostic-");
for (const entry of manifest.results) {
  const descriptor = JSON.parse(await fs.readFile(`${source}/${entry.file}`));
  const geometry = descriptor.asset_geometry;
  const lengths = geometry.lifts.map((lift) => lift.doors.length);
  for (const lift of geometry.lifts)
    lift.doors.push(...geometry.doors.filter((door) => door.sector_in === lift.motion_area_index));
  compileLiftApproaches(geometry);
  geometry.lifts.forEach((lift, index) => (lift.doors.length = lengths[index]));
  for (const door of geometry.doors) {
    if (!door.world_endpoints) continue;
    for (const side of ["in", "out"]) {
      const point = door[`point_${side}`];
      const receiver = geometry.sight_obstacles.find(
        (obstacle) =>
          obstacle.projection_area?.[0] === door[`sector_${side}`] &&
          obstacle.projection_area?.[1] === door[`layer_${side}`] &&
          pointInGameplayPolygon(
            point,
            obstacle.points.map((p) => [p.x, p.y - p.z_top]),
            true,
          ),
      );
      if (!receiver) {
        const world = door.world_endpoints[side === "in" ? "inside" : "outside"];
        assert.deepEqual(point, [Math.round(world[0]), Math.round(world[1] - world[2])]);
        continue;
      }
      const anchors = receiver.projection_plane ?? receiver.points.map((p) => [p.x, p.y, p.z_top]);
      const z = planeHeight(heightPlane(anchors.map(([x, y, z]) => [x, y - z, z])), point);
      door.world_endpoints[side === "in" ? "inside" : "outside"] = [point[0], point[1] + z, z];
    }
    door.point_mid = [...door.point_in];
    door.world_endpoints.middle = [...door.world_endpoints.inside];
  }
  await fs.writeFile(`${output}/${entry.file}`, JSON.stringify(descriptor));
}
await fs.writeFile(
  `${output}/diagnostics.json`,
  JSON.stringify({
    ...manifest,
    source,
    diagnostic: "ordinary doors included in lift approach preparation; unpublished",
  }),
);
console.log(output);
