import clipping, { type Polygon } from "polygon-clipping";
import type { Point } from "./level.ts";
import type { HeightPlane } from "./gameplay-plane.ts";
import { normalizeGeneratedMotion } from "./normalize-generated-motion.ts";
import { simplifyMotionRing, quantizeGeneratedMotionPolygon } from "./motion-quantization.ts";
import { preserveMovementBoundary } from "./preserve-movement-boundary.ts";
import { closeNavigationSeams } from "./close-navigation-seams.ts";

export interface NavigationPiece {
  plane: HeightPlane;
  layer: number;
  lift?: string;
  navigationRegion?: string;
  preserveMovementBoundary?: boolean;
  closeDeformationSeams?: boolean;
  polygon: Point[];
  blockers: Point[][];
}
export interface NavigationRegion {
  layer: number;
  lift?: string;
  polygon: Point[];
  blockers: Point[][];
  pieces: NavigationPiece[];
}
const shape = (p: NavigationPiece): Polygon => [p.polygon, ...p.blockers];
function movementRing(points: Point[], minimumArea = 0.5): Point[] {
  const ring = simplifyMotionRing(points);
  const area = ring.reduce((sum, p, i) => {
    const q = ring[(i + 1) % ring.length]!;
    return sum + p[0] * q[1] - q[0] * p[1];
  }, 0);
  if (ring.length < 3 || Math.abs(area) < minimumArea * 2)
    throw new Error(`Joined navigation region has a degenerate contour: ${JSON.stringify(points)}`);
  // Movement obstacles use the same winding as outer movement boundaries.
  if (area < 0) ring.reverse();
  return ring;
}

/** Explicit local regions share navigation topology while retaining every receiving plane. */
export function assembleNavigationRegions(
  pieces: NavigationPiece[],
  warnings: string[],
): NavigationRegion[] {
  const groups = new Map<string, NavigationPiece[]>();
  for (const [i, piece] of pieces.entries()) {
    const key = piece.lift
      ? `lift:${piece.lift}`
      : piece.navigationRegion
        ? `region:${piece.navigationRegion}`
        : `piece:${i}`;
    const group = groups.get(key) ?? [];
    group.push(piece);
    groups.set(key, group);
  }
  return [...groups.values()]
    .flatMap((members): NavigationRegion[] => {
      const first = members[0]!;
      if (
        members.length > 1 &&
        members.some((m) => m.preserveMovementBoundary) &&
        members.some((m) => !m.preserveMovementBoundary)
      )
        throw new Error("Joined navigation pieces must agree on movement boundary preservation");
      const layer = Math.min(...members.map((p) => p.layer));
      if (members.length === 1)
        return [
          {
            layer,
            lift: first.lift,
            polygon: first.polygon,
            blockers: first.blockers,
            pieces: members,
          },
        ];
      if (first.preserveMovementBoundary) {
        const boundaries = clipping.union(members.map((m): Polygon => [m.polygon]));
        // Another surface may provide a route through a cutout that extends
        // beyond its own partition. Preserve only the part no surface opens.
        const cutouts = members.flatMap((m) => {
          const otherFree = members
            .filter((other) => other !== m)
            .flatMap((other) =>
              other.blockers.length
                ? clipping.difference(
                    [other.polygon],
                    other.blockers.map((b) => [b]),
                  )
                : [[other.polygon]],
            );
          return m.blockers.flatMap((blocker) => clipping.difference([blocker], otherFree));
        });
        return boundaries.map((boundary) => {
          const preserved = preserveMovementBoundary(
            boundary[0]!,
            [...boundary.slice(1).map((hole): Polygon => [hole]), ...cutouts],
            warnings,
          );
          return {
            layer,
            lift: first.lift,
            ...preserved,
            pieces: members.filter(
              (member) => clipping.intersection([member.polygon], boundary).length > 0,
            ),
          };
        });
      }
      const joined = clipping.union(shape(first), ...members.slice(1).map(shape));
      const merged = normalizeGeneratedMotion(
        members.every((m) => m.closeDeformationSeams) ? closeNavigationSeams(joined) : joined,
        "Joined navigation region",
        warnings,
      );
      if (first.lift && merged.length !== 1)
        throw new Error(
          `Lift ${first.lift}: joined surfaces must form one connected traversal area`,
        );
      return merged.map((region) => ({
        layer,
        lift: first.lift,
        polygon: movementRing(region[0]!),
        blockers: region.slice(1).map((hole) => movementRing(hole)),
        pieces: members.flatMap((member) =>
          clipping.intersection(shape(member), region).flatMap((part) => {
            // Intersections can leave fractional slivers at a rounded union edge.
            // A receiver that collapses to a point or line on the movement grid
            // cannot own a playable pixel; omit it without changing navigation.
            if (
              !quantizeGeneratedMotionPolygon(
                part,
                Math.round,
                "Joined receiving fragment",
                warnings,
              )
            )
              return [];
            return [
              {
                ...member,
                polygon: movementRing(part[0]!, 1e-7),
                blockers: part.slice(1).map((hole) => movementRing(hole, 1e-7)),
              },
            ];
          }),
        ),
      }));
    })
    .sort((a, b) => a.layer - b.layer);
}
