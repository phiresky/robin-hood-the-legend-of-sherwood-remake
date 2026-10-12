// Generate integer-span candidates for native traversal review, without publication.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { pointInGameplayPolygon as inside } from "../shared/src/navigation-anchor.ts";
import { createRequire } from "node:module";
import { supportedFootprintCenters } from "../shared/src/supported-footprint-centers.ts";
import {
  jumpSpanInsideReceiver,
  jumpSpanHasFootprint,
  jumpFlightObstacles,
} from "../shared/src/jump-span-support.ts";
import { createJumpClearance } from "../shared/src/jump-clearance.ts";
import { auditCompiledJump } from "../shared/src/audit-compiled-jump.ts";

const clipping = createRequire(new URL("../shared/package.json", import.meta.url))(
  "polygon-clipping",
);

const args = process.argv.slice(2);
const requireClearFlight = args.includes("--require-clear-flight");
const checkFlight = args.includes("--flight-clearance") || requireClearFlight;
const maximumArguments = args.filter((argument) => argument.startsWith("--max-reviewed="));
assert.ok(maximumArguments.length <= 1);
const maximumReviewed = maximumArguments.length ? Number(maximumArguments[0].slice(15)) : 256;
assert.ok(Number.isInteger(maximumReviewed) && maximumReviewed > 0 && maximumReviewed <= 5000);
const limitArguments = args.filter((argument) => argument.startsWith("--limit="));
assert.ok(limitArguments.length <= 1);
const limit = limitArguments.length ? Number(limitArguments[0].slice(8)) : 24;
assert.ok(Number.isInteger(limit) && limit >= 1 && limit <= 24);
const [file, pairArgument = "0"] = args.filter(
  (argument) =>
    !argument.startsWith("--limit=") &&
    !argument.startsWith("--max-reviewed=") &&
    argument !== "--flight-clearance" &&
    argument !== "--require-clear-flight",
);
assert.ok(file, "Supply an exported descriptor and optional jump pair index");
const pairIndex = Number(pairArgument);
assert.ok(Number.isInteger(pairIndex) && pairIndex >= 0);
const descriptor = JSON.parse(await fs.readFile(file));
const geometry = descriptor.asset_geometry;
const pair = geometry.jump_line_pairs[pairIndex];
assert.ok(pair);
const lines = [pair.line1, pair.line2];
const zones = lines.map((_, side) => geometry.jump_zones[lines[1 - side].jump_zone_index]);
const centers = lines.map((line) => line.point_a.map((v, axis) => (v + line.point_b[axis]) / 2));
const worldCenters = centers.map(([x, y, z]) => [x, y + z, z]);
const freeAreas = [];
const motionAreas = new Map();
let sector = 0;
for (const [layer, entries] of geometry.motion_data.layers.entries())
  for (const entry of entries) {
    motionAreas.set(`${sector}/${layer}`, entry);
    sector += 1 + entry.obstacles.length;
  }
const sides = lines.map((_, side) => {
  const zone = zones[side],
    center = centers[side];
  const points = new Map();
  const motion = motionAreas.get(`${zone.sector}/${zone.layer}`);
  assert.ok(motion);
  if (motion.state_id !== 0) return points;
  const contour = motion.precise_polygon ?? motion.polygon.points;
  const free = clipping.difference([contour], ...motion.obstacles.map((o) => [o.polygon.points]));
  freeAreas[side] = free;
  const supportedCenters = supportedFootprintCenters([[contour]], free, [6, 3]);
  for (const [receiverIndex, receiver] of geometry.sight_obstacles.entries()) {
    if (
      !Array.isArray(receiver.projection_area) ||
      receiver.projection_area[0] !== zone.sector ||
      receiver.projection_area[1] !== zone.layer
    )
      continue;
    const plane = heightPlane(
      receiver.projection_plane ?? receiver.points.slice(0, 3).map((p) => [p.x, p.y, p.z_top]),
    );
    if (Math.abs(1 - plane[1]) < 1e-10) continue;
    const boundary = receiver.points.map((p) => [p.x, p.y]);
    for (let x = Math.ceil(center[0] - 32); x <= center[0] + 32; x++)
      for (let y = Math.ceil(center[1] - 32); y <= center[1] + 32; y++) {
        if (
          !supportedCenters.some(
            (region) =>
              inside([x, y], region[0], true) &&
              region.slice(1).every((hole) => !inside([x, y], hole, true)),
          )
        )
          continue;
        const worldY = (y + plane[0] * x + plane[2]) / (1 - plane[1]);
        if (!inside([x, worldY], boundary, true)) continue;
        const z = planeHeight(plane, [x, worldY]);
        const distance = [x, worldY, z].reduce(
          (sum, n, axis) => sum + (n - worldCenters[side][axis]) ** 2,
          0,
        );
        if (distance > 48 ** 2) continue;
        const key = `${x}/${y}`,
          previous = points.get(key);
        if (!previous || distance < previous.distance)
          points.set(key, {
            screen: [x, y, Math.ceil(z - 1e-4)],
            world: [x, worldY, z],
            distance,
            receiverIndex,
            boundary,
            plane,
          });
      }
  }
  return points;
});
const constrained = sides[0].size <= sides[1].size ? 0 : 1;
const other = 1 - constrained;
const vectors = new Map();
for (const a of sides[constrained].values())
  for (let dx = -8; dx <= 8; dx++)
    for (let dy = -8; dy <= 8; dy++) {
      if ((!dx && !dy) || Math.hypot(dx, dy) > 10) continue;
      const b = sides[constrained].get(`${a.screen[0] + dx}/${a.screen[1] + dy}`);
      if (!b || b.receiverIndex !== a.receiverIndex) continue;
      const direction = lines[constrained].point_a
        .slice(0, 2)
        .map((v, axis) => lines[constrained].point_b[axis] - v);
      if (dx * direction[0] + dy * direction[1] <= 0) continue;
      const midpoint = a.world.slice(0, 2).map((v, axis) => (v + b.world[axis]) / 2);
      if (!inside(midpoint, a.boundary, true)) continue;
      const key = `${dx}/${dy}`,
        entries = vectors.get(key) ?? [];
      entries.push([a, b]);
      vectors.set(key, entries);
    }
const candidates = [];
// Large endpoint height variation is amplified by the native paired-line
// landing convention. Compare nearby level spans before equally near steep ones.
const edgeScore = ([a, b]) => a.distance + b.distance + 100 * (a.screen[2] - b.screen[2]) ** 2;
for (const [key, entries] of vectors) {
  const [dx, dy] = key.split("/").map(Number);
  const opposites = [];
  for (const a of sides[other].values()) {
    const b = sides[other].get(`${a.screen[0] - dx}/${a.screen[1] - dy}`);
    if (!b || b.receiverIndex !== a.receiverIndex) continue;
    const midpoint = a.world.slice(0, 2).map((v, axis) => (v + b.world[axis]) / 2);
    if (inside(midpoint, a.boundary, true)) opposites.push([a, b]);
  }
  opposites.sort((a, b) => edgeScore(a) - edgeScore(b));
  for (const edge of entries)
    for (const opposite of opposites.slice(0, 3)) {
      const endpoints = [];
      endpoints[constrained] = edge;
      endpoints[other] = opposite;
      const [a, b] = endpoints[0].map((p) => p.screen);
      const target = endpoints[1][1].screen;
      if (-(b[1] - a[1]) * (target[0] - a[0]) + (b[0] - a[0]) * (target[1] - a[1]) <= 0) continue;
      candidates.push({
        endpoints,
        score: endpoints.reduce((sum, edge) => sum + edgeScore(edge), 0),
      });
    }
}
candidates.sort((a, b) => a.score - b.score);
const supported = [];
for (const candidate of candidates) {
  if (
    !candidate.endpoints.every(
      ([a, b], side) =>
        jumpSpanInsideReceiver(a.world.slice(0, 2), b.world.slice(0, 2), a.boundary) &&
        jumpSpanHasFootprint(a.screen.slice(0, 2), b.screen.slice(0, 2), freeAreas[side], [6, 3]),
    )
  )
    continue;
  supported.push(candidate);
  if (supported.length === (requireClearFlight ? maximumReviewed : limit)) break;
}
let flightContacts = new Map();
const flightObstacles = jumpFlightObstacles(geometry);
const flightClearance = checkFlight
  ? createJumpClearance(
      flightObstacles,
      new Map(
        zones.map((zone, side) => [
          String(side),
          {
            topology: { sector: zone.sector, layer: zone.layer },
            motionPolygon:
              motionAreas.get(`${zone.sector}/${zone.layer}`).precise_polygon ??
              motionAreas.get(`${zone.sector}/${zone.layer}`).polygon.points,
          },
        ]),
      ),
      ({ obstacle, range, reverse, points, paddingAxes, flightOrders, planeBounds }) => {
        const receiver = flightObstacles.indexOf(obstacle);
        const key = `${receiver}/${reverse}/${flightOrders ?? "path"}`,
          previous = flightContacts.get(key);
        if (previous) {
          previous.count++;
          previous.parameterBounds[0] = Math.min(previous.parameterBounds[0], range[0]);
          previous.parameterBounds[1] = Math.max(previous.parameterBounds[1], range[1]);
        } else
          flightContacts.set(key, {
            receiver,
            reverse,
            flightOrders,
            count: 1,
            parameterBounds: [...range],
            sample: { range, points, paddingAxes, planeBounds },
          });
      },
    )
  : undefined;
const output = await fs.mkdtemp("work/needs-review/map-compile/supported-jump-spans-");
const results = [];
const rejected = [];
const report = (complete) =>
  fs.writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({
      complete,
      scope: "static-geometry-only-not-gameplay-parity",
      source: file,
      pairIndex,
      checkFlight,
      requireClearFlight,
      maximumReviewed,
      candidates: candidates.length,
      supportedCandidates: supported.length,
      pointCounts: sides.map((points) => points.size),
      results,
      rejected,
    }),
  );
console.log(output);
await report(false);
for (const [index, candidate] of supported.entries()) {
  const document = structuredClone(descriptor);
  document.asset_geometry.jump_zones = zones;
  document.asset_geometry.jump_line_pairs = [
    {
      jump_long: pair.jump_long,
      ...Object.fromEntries(
        candidate.endpoints.map(([a, b], side) => [
          `line${side + 1}`,
          {
            point_a: a.screen,
            point_b: b.screen,
            jump_zone_index: 1 - side,
          },
        ]),
      ),
    },
  ];
  const name = `candidate-${index}.level.json`;
  flightContacts = new Map();
  const flightIssues = flightClearance
    ? auditCompiledJump(document.asset_geometry.jump_line_pairs[0], flightClearance)
    : undefined;
  if (requireClearFlight && flightIssues.length) {
    rejected.push({ index, score: candidate.score, flightIssues });
    if (rejected.length % 16 === 0) {
      await report(false);
      console.log(`Reviewed ${index + 1}/${supported.length}; ${results.length} clear candidates`);
    }
    continue;
  }
  await fs.writeFile(`${output}/${name}`, JSON.stringify(document));
  results.push({
    file: name,
    score: candidate.score,
    endpoints: candidate.endpoints,
    flightIssues,
    flightContacts: checkFlight ? [...flightContacts.values()] : undefined,
  });
  if (results.length === limit) break;
}
await report(results.length > 0);
assert.ok(
  results.length,
  requireClearFlight
    ? `No flight-clear candidate among ${rejected.length} reviewed supported spans; see ${output}/diagnostics.json`
    : "No representable integer span candidates near this connection",
);
console.log(
  JSON.stringify({
    candidates: candidates.length,
    reviewed: results.length + rejected.length,
    clear: checkFlight ? results.filter((r) => !r.flightIssues.length).length : undefined,
  }),
);
