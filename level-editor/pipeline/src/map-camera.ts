// Recover the map's pre-render camera elevation from level data.
//
// Sight-obstacle footprints are rectangles in the true ground plane but the
// level stores them in projected units (y foreshortened by sin(elevation)).
// Un-foreshortening by the right factor makes adjacent footprint edges
// perpendicular again, so we pick the factor minimising mean |cos(angle)|
// over all 4-point obstacles.
import type { MapCamera, ProtoLevel } from "@rle/shared";
import { cameraFromFit } from "@rle/shared";

/**
 * The game's projection constant: the original game's aspect ratio
 * 0.573576436351046 = cos 55° = sin 35°. The camera looks down at 55° from
 * the vertical, i.e. 35° above the ground; ground y is foreshortened by it.
 */
export const GAME_ASPECT_RATIO = 0.573576436351046;

/**
 * The map camera. The elevation is the game constant; the footprint fit is
 * run as a sanity check and only wins when a map is clearly built with a
 * different camera (more than 0.01 away in sin).
 */
export function fitMapCamera(level: ProtoLevel): MapCamera & { fit_cos: number; quads: number; fit_sin: number } {
  const fit = fitElevationFromFootprints(level);
  if (Math.abs(fit.s - GAME_ASPECT_RATIO) <= 0.01) return { ...cameraFromFit(GAME_ASPECT_RATIO), fit_cos: fit.err, quads: fit.quads, fit_sin: fit.s };
  console.warn(`map camera: footprint fit sin ${fit.s.toFixed(4)} differs from the game constant ${GAME_ASPECT_RATIO.toFixed(4)}; using the fit`);
  return { ...cameraFromFit(fit.s), fit_cos: fit.err, quads: fit.quads, fit_sin: fit.s };
}

/** grid search for the ground foreshortening that makes 4-point footprints rectangular */
function fitElevationFromFootprints(level: ProtoLevel): { s: number; err: number; quads: number } {
  const quads = level.sight_obstacles.filter((o) => o.points.length === 4);
  if (quads.length < 20) {
    throw new Error(`only ${quads.length} 4-point sight obstacles; cannot fit camera elevation`);
  }
  let best: { s: number; err: number } | null = null;
  for (let s = 0.3; s <= 1.0001; s += 0.0025) {
    let err = 0;
    let n = 0;
    for (const o of quads) {
      const p = o.points.map((q) => [q.x, q.y / s] as const);
      for (let i = 0; i < 4; i++) {
        const a = p[i]!,
          b = p[(i + 1) % 4]!,
          c = p[(i + 2) % 4]!;
        const e1 = [b[0] - a[0], b[1] - a[1]];
        const e2 = [c[0] - b[0], c[1] - b[1]];
        const l1 = Math.hypot(e1[0]!, e1[1]!);
        const l2 = Math.hypot(e2[0]!, e2[1]!);
        if (l1 < 5 || l2 < 5) continue;
        err += Math.abs((e1[0]! * e2[0]! + e1[1]! * e2[1]!) / (l1 * l2));
        n++;
      }
    }
    const mean = err / n;
    if (!best || mean < best.err) best = { s, err: mean };
  }
  return { s: best!.s, err: best!.err, quads: quads.length };
}
