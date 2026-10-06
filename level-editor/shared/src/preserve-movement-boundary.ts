import clipping, { type MultiPolygon } from "polygon-clipping";
import type { Point } from "./level.ts";
import { fixedPolygonBoolean } from "./fixed-polygon-boolean.ts";
import { normalizeGeneratedMotion } from "./normalize-generated-motion.ts";
import { simplifyMotionRing, quantizeGeneratedMotionPolygon } from "./motion-quantization.ts";
import { partitionMovementObstacles } from "./partition-movement-obstacles.ts";
import { assembleMovementContour } from "./assemble-movement-contour.ts";

/** Motion obstacles may cross the outer boundary. Keep both contours so their
 * fractional intersection remains implicit in the runtime's containment queries. */
export function preserveMovementBoundary(
  boundary: Point[],
  cutouts: MultiPolygon,
  warnings: string[],
  contourGroups?: (string | undefined)[],
) {
  const quantized = quantizeGeneratedMotionPolygon(
    [boundary],
    Math.round,
    "Preserved movement boundary",
    warnings,
  );
  if (!quantized) throw new Error("Preserved movement boundary collapsed on the movement grid");
  const outer = simplifyMotionRing(quantized[0]!);
  if (contourGroups && contourGroups.length !== cutouts.length)
    throw new Error("Movement contour labels do not match the cutouts");
  const groups = new Map<string | undefined, MultiPolygon>();
  for (const [index, cutout] of cutouts.entries()) {
    const key = contourGroups?.[index];
    const group = groups.get(key) ?? [];
    group.push(cutout);
    groups.set(key, group);
  }
  const blockers: Point[][] = [];
  const preciseBlockers: Point[][] = [];
  for (const [label, group] of groups) {
    // Independent exclusions need no union: containment against any contour is
    // already their union, without creating fractional intersection vertices.
    // Only explicitly labelled fragments describe one contour to reassemble.
    const integerContour = (region: MultiPolygon[number]) =>
      region.every((ring) => ring.every(([x, y]) => Number.isInteger(x) && Number.isInteger(y)));
    // A redundant fractional cutout can round outside the complete integer
    // exclusion that covers it. Discard it before introducing those new corners.
    const blocked =
      label === undefined
        ? group.filter(
            (region) =>
              integerContour(region) ||
              !group.some(
                (reference) =>
                  integerContour(reference) && clipping.difference(region, reference).length === 0,
              ),
          )
        : assembleMovementContour(group);
    // Rounding an outside contact must not manufacture an inward-facing corner.
    // Clean clipping-grid noise only for generated, fractional contours; complete
    // integer contours retain their implicit fractional intersections.
    const overlapping = blocked.filter((region) => {
      const tolerance = region.every((ring) =>
        ring.every(([x, y]) => Number.isInteger(x) && Number.isInteger(y)),
      )
        ? 0
        : 2 / 1048576;
      return clipping
        .intersection([boundary], region)
        .some((overlap) => simplifyMotionRing(overlap[0]!, tolerance).length >= 3);
    });
    // Keep each complete source contour alongside its grid obstacle. Emission
    // verifies identical rounding before binding precise physical landings.
    // Holed contours need a matching partition and retain grid collision here.
    preciseBlockers.push(
      ...overlapping.filter((region) => region.length === 1).map((region) => region[0]!),
    );
    for (const region of overlapping.flatMap((contour) =>
      normalizeGeneratedMotion([contour], "Preserved movement obstacle", warnings),
    )) {
      if (!fixedPolygonBoolean("intersection", [outer], [region]).length) continue;
      blockers.push(...partitionMovementObstacles(region));
    }
  }
  return { polygon: outer, blockers, preciseBlockers };
}
