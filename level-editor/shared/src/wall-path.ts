import { Vector3, MathUtils } from "three";
import { gameToScene, type MapCamera } from "./scene.ts";
import type { LevelSpline } from "./splines.ts";

// Older saved choices that no longer supply an exterior-only corner model.
export const excludedCornerAssetIds = new Set([
  "leicester-east-moat-tower",
  "leicester-west-moat-tower",
  "nottingham-northwest-round-tower",
]);

export function wallCorners(path: LevelSpline, camera: MapCamera) {
  if (!path.cornerAsset || excludedCornerAssetIds.has(path.cornerAsset)) return [];
  const points = path.points.map((p) => new Vector3(...gameToScene(camera, ...p)));
  return points.flatMap((p, i) => {
    if ((!path.closed && (i === 0 || i === points.length - 1)) || path.cornerDisabled?.includes(i))
      return [];
    const incoming = p.clone().sub(points[(i + points.length - 1) % points.length]!);
    incoming.z = 0;
    incoming.normalize();
    const outgoing = points[(i + 1) % points.length]!.clone().sub(p);
    outgoing.z = 0;
    outgoing.normalize();
    if (MathUtils.radToDeg(incoming.angleTo(outgoing)) < (path.cornerMinAngle ?? 35)) return [];
    const direction = incoming.add(outgoing).normalize();
    return [
      {
        index: i,
        position: p,
        rotation:
          Math.atan2(direction.y, direction.x) + MathUtils.degToRad(path.cornerRotation ?? 0),
      },
    ];
  });
}

/** Both artwork and gameplay terminate runs at the same towers and sharp corners. */
export function wallRuns(path: LevelSpline, camera: MapCamera): LevelSpline[] {
  const corners = wallCorners(path, camera);
  let runs: LevelSpline[];
  if (!corners.length) runs = [{ ...path, cornerAsset: undefined }];
  else {
    const breaks = corners.map((c) => c.index),
      indices: number[][] = [];
    if (path.closed)
      for (let j = 0; j < breaks.length; j++) {
        const run = [breaks[j]!],
          end = breaks[(j + 1) % breaks.length]!;
        for (
          let i = (breaks[j]! + 1) % path.points.length;
          i !== end;
          i = (i + 1) % path.points.length
        )
          run.push(i);
        run.push(end);
        indices.push(run);
      }
    else {
      const stops = [0, ...breaks, path.points.length - 1];
      for (let j = 0; j < stops.length - 1; j++)
        indices.push(
          Array.from({ length: stops[j + 1]! - stops[j]! + 1 }, (_, k) => stops[j]! + k),
        );
    }
    runs = indices.map((run) => ({
      ...path,
      cornerAsset: undefined,
      closed: false,
      points: run.map((i) => path.points[i]!),
    }));
  }
  return runs.flatMap((run) =>
    run.curved === false && run.points.length > 2
      ? Array.from({ length: run.points.length - (run.closed ? 0 : 1) }, (_, i) => ({
          ...run,
          closed: false,
          points: [run.points[i]!, run.points[(i + 1) % run.points.length]!],
        }))
      : [run],
  );
}
