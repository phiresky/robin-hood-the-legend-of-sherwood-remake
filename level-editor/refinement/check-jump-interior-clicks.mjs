// Diagnose complete clicks independently of perpendicular edge-offset probes.
import fs from "node:fs/promises";
import path from "node:path";
import assert from "node:assert/strict";
import { prepareCorridorStates } from "../shared/src/compile-navigation-graph.ts";
import { pointInGameplayPolygon } from "../shared/src/navigation-anchor.ts";

const hidden = process.argv.includes("--hidden");
const [directory, indices] = process.argv.slice(2).filter((argument) => argument !== "--hidden");
assert.ok(directory, "Supply an export directory and optional comma-separated line indices");
const selected = indices?.split(",").map(Number);
assert.ok(!selected || selected.every((index) => Number.isInteger(index) && index >= 0));
const manifest = JSON.parse(await fs.readFile(path.join(directory, "diagnostics.json")));
assert.equal(manifest.complete, true);
const output = await fs.mkdtemp("work/map-compile/interior-jump-clicks-");
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
  const lines = geometry.jump_line_pairs.flatMap((pair) => [pair.line1, pair.line2]);
  const probes = [];
  const cache = new Map();
  const interior = (index) => {
    if (cache.has(index)) return cache.get(index);
    const line = lines[index],
      opposite = lines[index ^ 1];
    const zone = geometry.jump_zones[opposite.jump_zone_index];
    const area = areas.get(zone.sector);
    assert.ok(area);
    const contours = [area.polygon.points, ...(area.precise_polygon ? [area.precise_polygon] : [])];
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
    const distance = (point) => (point[0] - midpoint[0]) ** 2 + (point[1] - midpoint[1]) ** 2;
    candidates.sort((a, b) => distance(a) - distance(b));
    const point = candidates.find((point) =>
      [-1, 1].every((dx) =>
        [-1, 1].every((dy) => {
          const shifted = [point[0] + dx, point[1] + dy];
          return allowed.every((check) =>
            check(shifted, shifted).some((state) => (state & 0xaaaaaaaa) === 0),
          );
        }),
      ),
    );
    cache.set(index, point);
    return point;
  };
  for (const index of selected ?? lines.map((_, index) => index)) {
    assert.ok(index < lines.length, `${entry.file}: missing jump line ${index}`);
    const start = interior(index),
      goal = interior(index ^ 1);
    if (!start || !goal)
      unavailable.push({
        file: entry.file,
        line: index,
        missing: [!start && "start", !goal && "goal"].filter(Boolean),
      });
    else probes.push({ line: index, start, goal, ...(hidden ? { hidden: true } : {}) });
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
