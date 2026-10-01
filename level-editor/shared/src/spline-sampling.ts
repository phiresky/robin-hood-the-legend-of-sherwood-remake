import { CatmullRomCurve3, Curve, Vector3 } from "three";
import { gameToScene, type MapCamera } from "./scene.ts";
import type { LevelSpline } from "./splines.ts";

class StraightPathCurve extends Curve<Vector3> {
  private readonly lengths = [0];
  private readonly points: Vector3[];
  private readonly closed: boolean;
  constructor(points: Vector3[], closed: boolean) {
    super();
    this.points = points;
    this.closed = closed;
    for (let i = 0; i < this.sections; i++)
      this.lengths.push(this.lengths[i]! + points[i]!.distanceTo(points[(i + 1) % points.length]!));
    this.arcLengthDivisions = this.sections * Math.max(40, Math.ceil(256 / this.sections));
  }
  private get sections() {
    return this.closed ? this.points.length : this.points.length - 1;
  }
  getPoint(t: number, target = new Vector3()) {
    const scaled = Math.max(0, Math.min(1, t)) * this.sections;
    const i = Math.min(this.sections - 1, Math.floor(scaled));
    return target
      .copy(this.points[i]!)
      .lerp(this.points[(i + 1) % this.points.length]!, scaled - i);
  }
  getTangent(t: number, target = new Vector3()) {
    const i = Math.min(this.sections - 1, Math.floor(Math.max(0, t) * this.sections));
    return target
      .copy(this.points[(i + 1) % this.points.length]!)
      .sub(this.points[i]!)
      .normalize();
  }
  getLength() {
    return this.lengths.at(-1)!;
  }
  getUtoTmapping(u: number, distance?: number) {
    const d = Math.max(0, Math.min(this.getLength(), distance || u * this.getLength()));
    for (let i = 0; i < this.sections; i++) {
      const end = this.lengths[i + 1]!,
        start = this.lengths[i]!;
      if (d <= end && end > start) return (i + (d - start) / (end - start)) / this.sections;
    }
    return 1;
  }
}

export function splineCurve(path: LevelSpline, camera: MapCamera) {
  if (path.curved === false)
    return new StraightPathCurve(
      path.points.map((p) => new Vector3(...gameToScene(camera, ...p))),
      path.closed,
    );
  const curve = new CatmullRomCurve3(
    path.points.map((p) => new Vector3(...gameToScene(camera, ...p))),
    path.closed,
    "centripetal",
  );
  curve.arcLengthDivisions = Math.max(256, path.points.length * 40);
  curve.updateArcLengths();
  return curve;
}

/** Curve parameter, not normalized arc distance: endpoints must match their handles. */
export function splineSectionAt(path: LevelSpline, parameter: number) {
  const count = path.closed ? path.points.length : path.points.length - 1;
  const scaled = Math.max(0, Math.min(1, parameter)) * count;
  const section = Math.min(count - 1, Math.floor(scaled));
  return { section, fraction: scaled - section, next: (section + 1) % path.points.length };
}

export function splineWidthAt(path: LevelSpline, parameter: number) {
  const { section, fraction, next } = splineSectionAt(path, parameter);
  const a = path.pointWidths?.[section] ?? path.width;
  const b = path.pointWidths?.[next] ?? path.width;
  return a + (b - a) * fraction;
}

export function splineMaterialWeightsAt(
  path: LevelSpline,
  parameter: number,
): Record<string, number> {
  const { section, fraction, next } = splineSectionAt(path, parameter);
  const weights: Record<string, number> = {};
  for (const [index, weight] of [
    [section, 1 - fraction],
    [next, fraction],
  ] as const) {
    const mix = path.pointMaterialMixes?.[index] ?? {
      [path.pointMaterials?.[index] ?? (path.kind === "river" ? "water_still" : "path_dirt")]: 1,
    };
    for (const [id, value] of Object.entries(mix))
      weights[id] = (weights[id] ?? 0) + value * weight;
  }
  return weights;
}

export function sampleSpline(path: LevelSpline, camera: MapCamera) {
  const curve = splineCurve(path, camera),
    length = curve.getLength();
  const count = Math.min(4096, Math.max(8, Math.ceil(length / 12)));
  const parameters = Array.from({ length: count + 1 }, (_, i) => ({
    parameter: curve.getUtoTmapping(i / count, 0),
    distance: (i / count) * length,
  }));
  const sections = path.closed ? path.points.length : path.points.length - 1;
  if (path.curved === false) {
    // Always include sharp controls so a ribbon never cuts across a corner.
    parameters.length = 0;
    let distance = 0;
    for (let section = 0; section < sections; section++) {
      const start = section / sections,
        end = (section + 1) / sections;
      const segmentLength = curve.getPoint(start).distanceTo(curve.getPoint(end));
      const steps = Math.max(1, Math.ceil(segmentLength / Math.max(12, length / 4096)));
      for (let i = 0; i < steps; i++)
        parameters.push({
          parameter: (section + i / steps) / sections,
          distance: distance + (segmentLength * i) / steps,
        });
      distance += segmentLength;
    }
    parameters.push({ parameter: 1, distance: length });
  }
  return parameters.map(({ parameter, distance }) => {
    let tangent = curve.getTangent(parameter),
      lateralScale = 1;
    const control = Math.round(parameter * sections);
    if (
      path.curved === false &&
      Math.abs(parameter * sections - control) < 1e-9 &&
      (path.closed || (control > 0 && control < sections))
    ) {
      const incoming = curve
        .getTangent(((control - 0.5 + sections) % sections) / sections)
        .setZ(0)
        .normalize();
      const outgoing = curve
        .getTangent(((control + 0.5) % sections) / sections)
        .setZ(0)
        .normalize();
      const bisector = incoming.clone().add(outgoing);
      if (bisector.lengthSq() > 1e-8) {
        tangent = bisector.normalize();
        // Limit spikes at near-reversals while retaining square, constant-width joins.
        lateralScale = Math.min(4, 1 / Math.max(1e-8, tangent.dot(outgoing)));
      }
    }
    const { section, next, fraction } = splineSectionAt(path, parameter);
    return {
      position: curve.getPoint(parameter),
      tangent,
      lateralScale,
      width: splineWidthAt(path, parameter),
      heightOffset:
        (path.pointHeightOffsets?.[section] ?? 0) * (1 - fraction) +
        (path.pointHeightOffsets?.[next] ?? 0) * fraction,
      ...splineSectionAt(path, parameter),
      distance,
    };
  });
}
