import polygonClipping, { type MultiPolygon } from "polygon-clipping";
import { fixedPolygonBoolean } from "./fixed-polygon-boolean.ts";

/** Preserve ordinary clipping results; recover near-coincident sweep failures
 * with the same fixed-point precision used to construct solid slices. */
export function subtractMovementCollision(subject: MultiPolygon, cuts: MultiPolygon): MultiPolygon {
  try {
    return polygonClipping.difference(subject, cuts);
  } catch {
    return fixedPolygonBoolean("difference", subject, [cuts]);
  }
}
