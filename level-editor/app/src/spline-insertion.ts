import { gameToScene, type LevelSpline, type MapCamera, type Vec3 } from "@rle/shared";
import { splineCurve } from "./spline-geometry.ts";
import { splineMaterialWeightsAt } from "../../shared/src/spline-sampling.ts";

/** Locate the closest curve section in the map plane, including a closed path's seam. */
export function nearestSplineSection(path: LevelSpline, camera: MapCamera, position: Vec3) {
  const sections = path.closed ? path.points.length : path.points.length - 1;
  if (sections < 1) throw new Error("A path needs two points before insertion");
  const curve = splineCurve(path, camera);
  const [x, y] = gameToScene(camera, ...position);
  let best = { section: 0, fraction: 0.5 },
    distance = Infinity;
  for (let section = 0; section < sections; section++) {
    let previous = curve.getPoint(section / sections);
    for (let step = 1; step <= 48; step++) {
      const next = curve.getPoint((section + step / 48) / sections);
      const dx = next.x - previous.x,
        dy = next.y - previous.y;
      const lengthSquared = dx * dx + dy * dy;
      const along = lengthSquared
        ? Math.max(0, Math.min(1, ((x - previous.x) * dx + (y - previous.y) * dy) / lengthSquared))
        : 0;
      const candidate = (x - previous.x - dx * along) ** 2 + (y - previous.y - dy * along) ** 2;
      if (candidate < distance) {
        distance = candidate;
        best = { section, fraction: (step - 1 + along) / 48 };
      }
      previous = next;
    }
  }
  return best;
}

export function insertSplinePoint(
  path: LevelSpline,
  section: number,
  fraction: number,
  position?: Vec3,
): LevelSpline {
  const count = path.points.length,
    sections = path.closed ? count : count - 1;
  if (
    count >= 256 ||
    section < 0 ||
    section >= sections ||
    !Number.isInteger(section) ||
    !Number.isFinite(fraction)
  )
    throw new Error("Invalid path point insertion");
  const t = Math.max(0, Math.min(1, fraction));
  const a = path.points[section]!,
    b = path.points[(section + 1) % count]!;
  const mix = (a: number, b: number) => a + (b - a) * t;
  const at = section + 1;
  const insert = <T>(values: T[], value: T): T[] => [
    ...values.slice(0, at),
    value,
    ...values.slice(at),
  ];
  const blendNumbers = (values?: number[]) =>
    values && insert(values, mix(values[section]!, values[(section + 1) % count]!));
  return {
    ...path,
    points: insert(
      path.points,
      position
        ? [position[0], position[1], mix(a[2], b[2])]
        : (a.map((value, i) => mix(value, b[i]!)) as Vec3),
    ),
    pointWidths: blendNumbers(path.pointWidths),
    pointHeightOffsets: blendNumbers(path.pointHeightOffsets),
    pointMaterials:
      path.pointMaterials && insert(path.pointMaterials, path.pointMaterials[section]!),
    pointMaterialMixes: insert(
      path.points.map((_, i) => path.pointMaterialMixes?.[i] ?? null),
      splineMaterialWeightsAt(path, (section + t) / sections),
    ),
    cornerDisabled: path.cornerDisabled?.map((index) => (index >= at ? index + 1 : index)),
  };
}
