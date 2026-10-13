// Choose test passages through a changing footprint, clear of permanent props.
import { pointInGameplayPolygon } from "../shared/src/navigation-anchor.ts";

function edgeDistance(p, a, b) {
  const dx = b[0] - a[0],
    dy = b[1] - a[1];
  const square = dx * dx + dy * dy;
  const t = square
    ? Math.max(0, Math.min(1, ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / square))
    : 0;
  return Math.hypot(p[0] - a[0] - t * dx, p[1] - a[1] - t * dy);
}
export function selectPanelRoute(shape, permanent, height) {
  const polygon = shape.points.map((point) => [point.x, point.y - point.z_bottom]);
  const obstacles = permanent
    .filter((part) => part.points.some((point) => point.z_bottom < height + 40))
    .map((part) => part.points.map((point) => [point.x, point.y - point.z_bottom]));
  const inside = (point, ring) => pointInGameplayPolygon(point, ring, true);
  const edgeGap = (point, ring) =>
    Math.min(...ring.map((a, i) => edgeDistance(point, a, ring[(i + 1) % ring.length])));
  const xs = polygon.map((point) => point[0]),
    ys = polygon.map((point) => point[1]);
  const yMin = Math.min(...ys),
    yMax = Math.max(...ys),
    centerY = (yMin + yMax) / 2;
  const rows = [];
  for (let y = yMin + 2; y < yMax - 2; y += 2) rows.push(y);
  rows.sort((a, b) => Math.abs(a - centerY) - Math.abs(b - centerY));
  for (const y of rows) {
    const start = [Math.min(...xs) - 15, y],
      end = [Math.max(...xs) + 15, y];
    const steps = Math.ceil((end[0] - start[0]) / 2);
    const samples = Array.from({ length: steps + 1 }, (_, i) => [
      start[0] + ((end[0] - start[0]) * i) / steps,
      y,
    ]);
    if (!samples.some((point) => inside(point, polygon) && edgeGap(point, polygon) >= 10)) continue;
    if (
      samples.some((point) =>
        obstacles.some((ring) => inside(point, ring) || edgeGap(point, ring) < 12),
      )
    )
      continue;
    return { start, end };
  }
  throw new Error("No conservative passage around permanent geometry; author dedicated probes");
}
