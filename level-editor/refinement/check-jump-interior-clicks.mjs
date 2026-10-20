// Diagnose complete clicks independently of perpendicular edge-offset probes.
import fs from "node:fs/promises";
import path from "node:path";
import assert from "node:assert/strict";
import { prepareCorridorStates } from "../shared/src/compile-navigation-graph.ts";
import { pointInGameplayPolygon } from "../shared/src/navigation-anchor.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";

const hidden = process.argv.includes("--hidden");
const receiverSamples = process.argv.includes("--receiver-samples");
const sampleArguments = process.argv
  .slice(2)
  .filter((argument) => argument.startsWith("--samples="));
assert.ok(sampleArguments.length <= 1);
const samples = sampleArguments.length ? Number(sampleArguments[0].slice("--samples=".length)) : 1;
assert.ok(Number.isInteger(samples) && samples >= 1 && samples <= 16, "Sample count must be 1–16");
const [directory, indices] = process.argv
  .slice(2)
  .filter(
    (argument) =>
      argument !== "--hidden" &&
      argument !== "--receiver-samples" &&
      !argument.startsWith("--samples="),
  );
assert.ok(directory, "Supply an export directory and optional comma-separated line indices");
const selected = indices?.split(",").map(Number);
assert.ok(!selected || selected.every((index) => Number.isInteger(index) && index >= 0));
const manifest = JSON.parse(await fs.readFile(path.join(directory, "diagnostics.json")));
assert.equal(manifest.complete, true);
const output = await fs.mkdtemp("work/needs-review/map-compile/interior-jump-clicks-");
const results = [],
  unavailable = [];
for (const entry of manifest.results) {
  const bytes = await fs.readFile(path.join(directory, entry.file));
  const geometry = JSON.parse(bytes).asset_geometry;
  const areas = new Map();
  let sector = 0;
  for (const layer of geometry.motion_data.layers)
    for (const area of layer) {
      areas.set(sector, area);
      sector += 1 + area.obstacles.length;
    }
  const lines = (geometry.jump_line_pairs ?? []).flatMap((pair) => [pair.line1, pair.line2]);
  if (!lines.length) unavailable.push({ file: entry.file, missing: ["jump lines"] });
  const probes = [];
  const cache = new Map();
  const interior = (index) => {
    if (cache.has(index)) return cache.get(index);
    const line = lines[index],
      opposite = lines[index ^ 1];
    const zone = geometry.jump_zones[opposite.jump_zone_index];
    const area = areas.get(zone.sector);
    assert.ok(area);
    // Native loading replaces the integer contour when precise movement is present.
    const contours = [area.precise_polygon ?? area.polygon.points];
    const allowed = contours.map((contour) => prepareCorridorStates(area.obstacles, contour));
    const polygon = zone.polygon.points;
    const midpoint = line.point_a
      .slice(0, 2)
      .map((value, axis) => (value + line.point_b[axis]) / 2);
    const candidates = [];
    const minX = Math.floor(Math.min(...polygon.map((point) => point[0])));
    const maxX = Math.ceil(Math.max(...polygon.map((point) => point[0])));
    const minY = Math.floor(Math.min(...polygon.map((point) => point[1])));
    const maxY = Math.ceil(Math.max(...polygon.map((point) => point[1])));
    for (let x = minX; x <= maxX; x += 2)
      for (let y = minY; y <= maxY; y += 2) {
        const point = [x + 0.25, y + 0.25];
        if (pointInGameplayPolygon(point, polygon)) candidates.push(point);
      }
    if (receiverSamples) {
      for (const receiver of geometry.sight_obstacles) {
        if (
          receiver.projection_area?.[0] !== zone.sector ||
          receiver.projection_area?.[1] !== zone.layer
        )
          continue;
        const world = receiver.points.map((p) => [p.x, p.y]);
        const plane = heightPlane(
          receiver.projection_plane ?? receiver.points.slice(0, 3).map((p) => [p.x, p.y, p.z_top]),
        );
        const center = [0, 1].map(
          (axis) => world.reduce((sum, point) => sum + point[axis], 0) / world.length,
        );
        // An almost edge-on receiver can fall between every screen-grid sample.
        // Project actual floor points; do not expand its physical coverage.
        for (const vertex of world)
          for (const fraction of [0, 0.25, 0.5, 0.75]) {
            const point = center.map((value, axis) => value + fraction * (vertex[axis] - value));
            if (!pointInGameplayPolygon(point, world, true)) continue;
            const screen = [point[0], point[1] - planeHeight(plane, point)];
            if (pointInGameplayPolygon(screen, polygon)) candidates.push(screen);
          }
      }
    }
    const distance = (point) => (point[0] - midpoint[0]) ** 2 + (point[1] - midpoint[1]) ** 2;
    candidates.sort((a, b) => distance(a) - distance(b));
    const supported = (point) =>
      [-1, 1].every((dx) =>
        [-1, 1].every((dy) => {
          const shifted = [point[0] + dx, point[1] + dy];
          return allowed.every((check) =>
            check(shifted, shifted).some((state) => (state & 0xaaaaaaaa) === 0),
          );
        }),
      );
    const points = [];
    // Preserve the nearest-point default, then spread additional probes across
    // the usable zone by maximizing their distance from already sampled points.
    const remaining = [];
    const seen = new Set();
    for (const point of candidates) {
      const key = `${point[0]}/${point[1]}`;
      if (seen.has(key)) continue;
      seen.add(key);
      if (!supported(point)) continue;
      remaining.push(point);
      if (samples === 1) break;
    }
    if (remaining.length) points.push(remaining.shift());
    while (points.length < samples && remaining.length) {
      let bestIndex = 0,
        bestDistance = -1;
      for (const [i, point] of remaining.entries()) {
        const separation = Math.min(
          ...points.map((sample) => (sample[0] - point[0]) ** 2 + (sample[1] - point[1]) ** 2),
        );
        if (separation > bestDistance) {
          bestDistance = separation;
          bestIndex = i;
        }
      }
      points.push(remaining.splice(bestIndex, 1)[0]);
    }
    cache.set(index, points);
    return points;
  };
  for (const index of selected ?? lines.map((_, index) => index)) {
    assert.ok(index < lines.length, `${entry.file}: missing jump line ${index}`);
    const start = interior(index),
      goal = interior(index ^ 1);
    if (!start.length || !goal.length)
      unavailable.push({
        file: entry.file,
        line: index,
        missing: [!start.length && "start", !goal.length && "goal"].filter(Boolean),
      });
    else
      for (const from of start)
        for (const to of goal)
          probes.push({ line: index, start: from, goal: to, ...(hidden ? { hidden: true } : {}) });
  }
  await fs.writeFile(path.join(output, entry.file), bytes);
  results.push({ ...entry, click_probes: probes });
}
await fs.writeFile(
  path.join(output, "diagnostics.json"),
  JSON.stringify({
    complete: true,
    supportedFloorClicks: true,
    scope: "interior-native-footprint-probes-not-world-support-or-rendering-proof",
    source: directory,
    samplesPerSide: samples,
    receiverSamples,
    unavailable,
    results,
  }),
);
console.log(output);
console.log(
  JSON.stringify({
    probes: results.reduce((sum, entry) => sum + entry.click_probes.length, 0),
    unavailable,
  }),
);
if (unavailable.length) process.exitCode = 1;
