import type { NavigationConnection } from "./assemble-navigation-joins.ts";

type Point = [number, number];

function coordinate(value: number): number {
  const integer = Math.round(value);
  return Math.abs(value - integer) <= 8 * Number.EPSILON * Math.max(1, Math.abs(value))
    ? integer
    : value;
}

/** Give matched floors identical seam vertices before polygon union and clipping.
 * A socket can occupy only part of a long boundary. Independently rounding that
 * boundary and its neighbor otherwise turns a shared line into a narrow crack. */
export function alignNavigationBoundaries(
  surfaces: { navigationRegion?: string; polygon: Point[] }[],
  connections: NavigationConnection[],
) {
  const endpoints = new Map<string, Point[]>();
  for (const { from, to, overlap } of connections) {
    const [a, b] = from.edge;
    const points = overlap.map((t): Point => [
      coordinate(a[0] + t * (b[0] - a[0])),
      coordinate(a[1] - a[2] + t * (b[1] - b[2] - (a[1] - a[2]))),
    ]);
    for (const region of [from.region, to.region]) {
      const existing = endpoints.get(region) ?? [];
      existing.push(...points);
      endpoints.set(region, existing);
    }
  }
  for (const surface of surfaces) {
    const points = endpoints.get(surface.navigationRegion ?? "");
    if (!points) continue;
    const polygon = surface.polygon;
    surface.polygon = polygon.flatMap((a, i) => {
      const b = polygon[(i + 1) % polygon.length]!;
      const dx = b[0] - a[0],
        dy = b[1] - a[1];
      const length = Math.hypot(dx, dy);
      if (length < 1e-8) return [a];
      const candidates = points
        .flatMap((p) => {
          const t = ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / (length * length);
          const distance = Math.hypot(p[0] - a[0] - t * dx, p[1] - a[1] - t * dy);
          return t >= -1e-7 && t <= 1 + 1e-7 && distance < 1e-4 ? [{ t, p }] : [];
        })
        .sort((left, right) => left.t - right.t);
      const start = candidates.find(({ p }) => Math.hypot(p[0] - a[0], p[1] - a[1]) < 1e-4)?.p ?? a;
      const result = [start];
      for (const { p } of candidates) {
        const previous = result[result.length - 1]!;
        if (
          Math.hypot(p[0] - previous[0], p[1] - previous[1]) < 1e-4 ||
          Math.hypot(p[0] - b[0], p[1] - b[1]) < 1e-4
        )
          continue;
        result.push(p);
      }
      return result;
    });
  }
}
