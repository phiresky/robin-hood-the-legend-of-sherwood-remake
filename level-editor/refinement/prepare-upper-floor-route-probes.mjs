// Select conservative, reproducible walking probes inside exported upper receivers.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { pointInGameplayPolygon } from "../shared/src/navigation-anchor.ts";

const [directory, filename, minimumHeightText = "500"] = process.argv.slice(2);
assert.ok(directory && filename);
const minimumHeight = Number(minimumHeightText);
assert.ok(Number.isFinite(minimumHeight));
const descriptor = JSON.parse(await fs.readFile(`${directory}/${filename}`));
const receivers = descriptor.asset_geometry.sight_obstacles.filter(
  (receiver) =>
    receiver.projection_area && receiver.points.every((point) => point.z_top > minimumHeight),
);
assert.ok(receivers.length, "No upper receivers selected");
const distanceToEdge = (p, a, b) => {
  const dx = b[0] - a[0],
    dy = b[1] - a[1];
  const lengthSquared = dx * dx + dy * dy;
  const t =
    lengthSquared === 0
      ? 0
      : Math.max(0, Math.min(1, ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / lengthSquared));
  return Math.hypot(p[0] - a[0] - t * dx, p[1] - a[1] - t * dy);
};
const probes = [];
for (const receiver of receivers) {
  const polygon = receiver.points.map((point) => [point.x, point.y - point.z_top]);
  const xs = polygon.map((point) => point[0]),
    ys = polygon.map((point) => point[1]);
  const safe = (point) =>
    pointInGameplayPolygon(point, polygon, false) &&
    polygon.every((a, i) => distanceToEdge(point, a, polygon[(i + 1) % polygon.length]) >= 12);
  let best;
  for (let y = Math.min(...ys); y <= Math.max(...ys); y += 2) {
    let start;
    for (let x = Math.min(...xs); x <= Math.max(...xs) + 2; x += 2) {
      if (safe([x, y])) {
        start ??= [x, y];
        const length = x - start[0];
        if (!best || length > best.length) best = { start, end: [x, y], length };
      } else start = undefined;
    }
  }
  assert.ok(best && best.length >= 20, "Receiver has no conservative walking span");
  for (const reverse of [false, true])
    probes.push({
      start: reverse ? best.end : best.start,
      end: reverse ? best.start : best.end,
      sector: receiver.projection_area[0],
      layer: receiver.projection_area[1],
      max_length: best.length + 1,
    });
}
await fs.writeFile(
  `${directory}/diagnostics.json`,
  JSON.stringify({
    scope: "upper-floor-sprite-routes-not-visual-contact-proof",
    complete: true,
    results: [{ file: filename, route_probes: probes }],
  }),
  { flag: "wx" },
);
console.log(JSON.stringify({ receivers: receivers.length, routes: probes.length }));
