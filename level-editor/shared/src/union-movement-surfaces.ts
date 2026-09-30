import clipping, { type MultiPolygon, type Polygon } from "polygon-clipping";
import { fixedPolygonBoolean } from "./fixed-polygon-boolean.ts";

/** Preserve existing contour ordering, retrying numerical sweep failures at fixed precision. */
export function unionMovementSurfaces(
  input: Polygon[],
  label: string,
  warnings: string[],
): MultiPolygon {
  try {
    return clipping.union(input[0]!, ...input.slice(1));
  } catch (error) {
    if (
      !(error instanceof Error) ||
      !/Unable to find segment .* in SweepLine tree|Infinite loop when passing sweep line/.test(
        error.message,
      )
    )
      throw error;
    const result = fixedPolygonBoolean("union", input[0]!, input.slice(1));
    warnings.push(`${label}: merged near-coincident surface edges using fixed-point clipping.`);
    return result;
  }
}
