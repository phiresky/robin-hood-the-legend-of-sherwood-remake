import type { NavigationRegion } from "./assemble-navigation-regions.ts";
import { fixedPolygonBoolean } from "./fixed-polygon-boolean.ts";
import { planeHeight } from "./gameplay-plane.ts";

/** A single native region cannot distinguish two receiving heights at the same projected point. */
export function warnSelfOverlappingNavigation(
  regions: NavigationRegion[],
  warnings: string[],
): void {
  for (const region of regions) {
    if (region.lift || region.pieces.length < 2) continue;
    const pieces = region.pieces
      .map((piece) => {
        const polygon = piece.receivingPolygon ?? piece.polygon;
        return {
          piece,
          polygon,
          minX: Math.min(...polygon.map((p) => p[0])),
          maxX: Math.max(...polygon.map((p) => p[0])),
          minY: Math.min(...polygon.map((p) => p[1])),
          maxY: Math.max(...polygon.map((p) => p[1])),
        };
      })
      .sort((a, b) => a.minX - b.minX);
    const coverage = new Map<(typeof pieces)[number], ReturnType<typeof fixedPolygonBoolean>>();
    const free = (item: (typeof pieces)[number]) => {
      let result = coverage.get(item);
      if (!result) {
        result = item.piece.blockers.length
          ? fixedPolygonBoolean(
              "difference",
              [[item.polygon]],
              item.piece.blockers.map((hole) => [hole]),
            )
          : [[item.polygon]];
        coverage.set(item, result);
      }
      return result;
    };
    let active: typeof pieces = [];
    let reported = false;
    for (const current of pieces) {
      active = active.filter((other) => other.maxX > current.minX);
      for (const other of active) {
        if (current.minY >= other.maxY || current.maxY <= other.minY) continue;
        if (current.piece.plane.every((n, i) => Math.abs(n - other.piece.plane[i]!) < 1e-7))
          continue;
        const overlap = fixedPolygonBoolean("intersection", free(current), [free(other)]);
        const point = overlap
          .flatMap((p) => p[0]!)
          .find(
            (p) =>
              Math.abs(planeHeight(current.piece.plane, p) - planeHeight(other.piece.plane, p)) >
              1e-4,
          );
        if (!point) continue;
        const name = current.piece.navigationRegion ?? other.piece.navigationRegion ?? "unnamed";
        warnings.push(
          `Navigation region ${name}: receiving floors overlap at different heights near projected (${point.map((n) => n.toFixed(2)).join(", ")}). One sector cannot distinguish these floors; separate the local navigation regions and connect them with passages. Geometry is retained, but this overlap is not gameplay-ready.`,
        );
        reported = true;
        break;
      }
      if (reported) break;
      active.push(current);
    }
  }
}
