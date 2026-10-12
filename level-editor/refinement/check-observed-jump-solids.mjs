// Compare recorded airborne actor anchors with the exported initial solid geometry.
import fs from "node:fs/promises";
import path from "node:path";
import assert from "node:assert/strict";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { pointInGameplayPolygon } from "../shared/src/navigation-anchor.ts";

const [directory, reportName = "actor-jump-click-report.json"] = process.argv.slice(2);
assert.ok(directory, "Supply a native audit directory recorded with ROBIN_TRACE_JUMP=1");
const report = JSON.parse(await fs.readFile(path.join(directory, reportName)));
assert.equal(report.complete, true, "Complete the movement audit before reviewing its traces");
const byFile = new Map();
const hits = [];
let samples = 0;
for (const result of report.results) {
  assert.ok(result.arrival?.trajectory?.length, "Recorded native positions are required");
  let solids = byFile.get(result.file);
  if (!solids) {
    const geometry = JSON.parse(
      await fs.readFile(path.join(directory, result.file)),
    ).asset_geometry;
    assert.ok(
      !geometry.movement_transitions?.length,
      "Trace must include state changes before reviewing a map with changing solids",
    );
    solids = geometry.sight_obstacles.flatMap((receiver, index) => {
      if (!receiver.solid || receiver.initial_active === false) return [];
      return [
        {
          index,
          boundary: receiver.points.map((point) => [point.x, point.y]),
          top: heightPlane(
            receiver.projection_plane ??
              receiver.points.slice(0, 3).map((point) => [point.x, point.y, point.z_top]),
          ),
          bottom: heightPlane(
            receiver.projection_plane ??
              receiver.points.slice(0, 3).map((point) => [point.x, point.y, point.z_bottom]),
          ),
        },
      ];
    });
    byFile.set(result.file, solids);
  }
  for (const [frame, recorded] of result.arrival.trajectory.entries()) {
    if (!/^Jumping(Long|LongSword|Up|Down)$/.test(recorded.action)) continue;
    assert.ok(recorded.position.length === 3 && recorded.position.every(Number.isFinite));
    samples++;
    const [x, y, z] = recorded.position;
    for (const solid of solids) {
      const top = planeHeight(solid.top, [x, y]);
      const bottom = planeHeight(solid.bottom, [x, y]);
      if (
        z >= top - 0.01 ||
        z <= bottom + 0.01 ||
        !pointInGameplayPolygon([x, y], solid.boundary, true)
      )
        continue;
      hits.push({
        file: result.file,
        line: result.line ?? result.probe?.line,
        t: result.t,
        frame,
        action: recorded.action,
        position: recorded.position,
        receiver: solid.index,
        penetration: top - z,
      });
    }
  }
}
assert.ok(samples, "No airborne positions in the supplied trace");
const summary = {
  complete: true,
  clear: hits.length === 0,
  scope: "observed-airborne-anchor-vs-static-solids-not-sprite-volume-or-rendering",
  reportName,
  samples,
  contactCount: hits.length,
  maximumPenetration: hits.reduce((maximum, hit) => Math.max(maximum, hit.penetration), 0),
};
await fs.writeFile(
  path.join(directory, "observed-solid-contact.json"),
  JSON.stringify({ ...summary, hits }),
);
console.log(JSON.stringify(summary));
if (hits.length) process.exitCode = 1;
