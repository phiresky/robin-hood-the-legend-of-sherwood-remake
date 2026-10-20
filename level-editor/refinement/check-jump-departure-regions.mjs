// Diagnostic only: find translations supporting a complete departure on one convex receiver.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createRequire } from "node:module";
import { createHash } from "node:crypto";

const polygonClipping = createRequire(new URL("../shared/package.json", import.meta.url))(
  "polygon-clipping",
);

const [directory] = process.argv.slice(2);
assert.ok(directory, "Supply a compiled placement diagnostics directory");
const manifest = JSON.parse(await fs.readFile(`${directory}/diagnostics.json`));
assert.equal(manifest.complete, true);
const results = [];
const cross = (a, b, c) =>
  (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0]);
for (const entry of manifest.results) {
  const bytes = await fs.readFile(`${directory}/${entry.file}`);
  const descriptor = JSON.parse(bytes);
  const descriptorSha256 = createHash("sha256").update(bytes).digest("hex");
  const geometry = descriptor.asset_geometry;
  for (const [pairIndex, pair] of geometry.jump_line_pairs.entries()) {
    if (!pair.jump_long) continue;
    for (const [side, source, opposite] of [
      [0, pair.line1, pair.line2],
      [1, pair.line2, pair.line1],
    ]) {
      if (Math.min(source.point_a[2], source.point_b[2]) <= 0) continue;
      const a = source.point_a.slice(0, 2), b = source.point_b.slice(0, 2);
      const dx = b[0] - a[0], dy = b[1] - a[1], length = Math.hypot(dx, dy);
      assert.ok(length > 0);
      const center = a.map((value, axis) => (value + b[axis]) / 2);
      const normal = [-dy / length, dx / length];
      const corners = [a, b, b.map((v, i) => v + 15 * normal[i]),
        a.map((v, i) => v + 15 * normal[i])];
      const zone = geometry.jump_zones[opposite.jump_zone_index];
      const receivers = geometry.sight_obstacles.filter(
        (face) => face.projection_area?.[0] === zone.sector &&
          face.projection_area?.[1] === zone.layer,
      );
      const regions = [];
      const depthLimits = [];
      let skippedNonconvex = 0;
      for (const receiver of receivers) {
        const ring = receiver.points.map((p) => [p.x, p.y - p.z_top]);
        const turns = ring.map((p, i) => cross(p, ring[(i + 1) % ring.length],
          ring[(i + 2) % ring.length])).filter((value) => Math.abs(value) > 1e-8);
        if (!turns.length || turns.some((value) => value * turns[0] < 0)) {
          skippedNonconvex++;
          continue;
        }
        // For a convex face, containing the four corners contains the entire strip.
        const shifted = corners.map((corner) =>
          [ring.map((p) => p.map((value, axis) => value - corner[axis]))]);
        regions.push(...polygonClipping.intersection(...shifted));
        const fits = (depth) => {
          const corners = [a, b, b.map((v, i) => v + depth * normal[i]),
            a.map((v, i) => v + depth * normal[i])];
          return polygonClipping.intersection(...corners.map((corner) =>
            [ring.map((p) => p.map((value, axis) => value - corner[axis]))])).length > 0;
        };
        let low = 0, high = 15;
        if (fits(15)) low = 15;
        else for (let iteration = 0; iteration < 24; iteration++) {
          const middle = (low + high) / 2;
          if (fits(middle)) low = middle;
          else high = middle;
        }
        depthLimits.push(low);
      }
      const feasible = regions.length ? polygonClipping.union(...regions) : [];
      const candidates = feasible.flatMap((polygon) => polygon.flatMap((ring) =>
        ring.slice(0, -1).map((a, i) => {
          const b = ring[i + 1], delta = b.map((v, axis) => v - a[axis]);
          const squared = delta.reduce((sum, v) => sum + v * v, 0);
          const t = squared ? Math.max(0, Math.min(1,
            -a.reduce((sum, v, axis) => sum + v * delta[axis], 0) / squared)) : 0;
          return a.map((v, axis) => v + t * delta[axis]);
        })));
      candidates.sort((a, b) => Math.hypot(...a) - Math.hypot(...b));
      results.push({
        file: entry.file, descriptorSha256, pair: pairIndex, side, center, normal,
        receiverCount: receivers.length, skippedNonconvex,
        maximumSupportedDepthUpTo15: Math.max(0, ...depthLimits),
        feasibleTranslationRegions: feasible,
        nearestBoundaryTranslation: candidates[0] ?? null,
      });
    }
  }
}
const output = `${directory}/jump-departure-regions.json`;
await fs.writeFile(output, JSON.stringify({
  scope: "single-convex-receiver-strip-coverage-only",
  limitations: "No height, approach, flight, integer-rounding or cross-placement proof; multi-face support may have additional solutions.",
  results,
}));
console.log(output);
