import type {
  AssetGameplay,
  AssetDoor,
  AssetWalkableSurface,
} from "../../shared/src/asset-gameplay.ts";
import type { Point } from "../../shared/src/level.ts";
import type { Vec3 } from "../../shared/src/scene.ts";

/** Authoring migration between translated local frames; never reads map data. */
export function translateGameplayFrames(
  source: AssetGameplay,
  offsets: ReadonlyMap<string, Vec3>,
): AssetGameplay {
  if (source.spline) throw new Error("Recalibrate spline geometry after changing gameplay frames");
  const result = structuredClone(source);
  const offset = (node: string): Vec3 => {
    const value = offsets.get(node);
    if (!value || !value.every(Number.isFinite))
      throw new Error(`Missing finite frame offset: ${node}`);
    return value;
  };
  const point = (node: string, p: Vec3): Vec3 => {
    const d = offset(node);
    return [p[0] + d[0], p[1] + d[1], p[2] + d[2]];
  };
  const xy = (node: string, p: Point): Point => {
    const d = offset(node);
    return [p[0] + d[0], p[1] + d[1]];
  };
  const line = (node: string, points: Vec3[]) => points.map((p) => point(node, p));
  const segment = (node: string, points: [Vec3, Vec3]): [Vec3, Vec3] => [
    point(node, points[0]),
    point(node, points[1]),
  ];
  const surface = (s: AssetWalkableSurface) => {
    const dz = offset(s.node)[2];
    s.polygon = s.polygon.map((p) => xy(s.node, p));
    s.height = typeof s.height === "number" ? s.height + dz : s.height.map((z) => z + dz);
    if (s.navigationHeight !== undefined) s.navigationHeight += dz;
    if (s.holes) s.holes = s.holes.map((hole) => hole.map((p) => xy(s.node, p)));
    if (s.navigationJoins)
      s.navigationJoins = s.navigationJoins.map((edge) => segment(s.node, edge));
    const material = s.projectionMaterials;
    if (material?.planePoints) {
      const [a, b, c] = material.planePoints;
      material.planePoints = [point(s.node, a), point(s.node, b), point(s.node, c)];
    }
    if (material?.footprint) material.footprint = line(s.node, material.footprint);
    if (material?.priorityHeight !== undefined) material.priorityHeight += dz;
  };
  const door = (d: AssetDoor) => {
    d.polygon = d.polygon.map((p) => xy(d.node, p));
    for (const key of ["outside", "inside", "middle"] as const) d[key] = point(d.node, d[key]);
    for (const key of ["outsideAnchor", "insideAnchor"] as const)
      if (d[key]) d[key] = point(d.node, d[key]);
    for (const key of ["outsideReceiverSegment", "insideReceiverSegment"] as const)
      if (d[key]) d[key] = segment(d.node, d[key]);
  };
  if (result.placementGroundHeight !== undefined) {
    const heights = new Set([...offsets.values()].map((d) => d[2]));
    if (heights.size !== 1)
      throw new Error("Placement height requires one shared vertical frame offset");
    result.placementGroundHeight += heights.values().next().value!;
  }
  for (const s of [
    ...result.surfaces,
    ...(result.movementBlockers ?? []),
    ...(result.movementClearances ?? []),
  ])
    surface(s);
  for (const volume of result.volumes ?? []) {
    const [dx, dy, dz] = offset(volume.node);
    volume.shape.points = volume.shape.points.map((p) => ({
      ...p,
      x: p.x + dx,
      y: p.y + dy,
      z_bottom: p.z_bottom + dz,
      z_top: p.z_top + dz,
    }));
  }
  for (const r of result.projectionReceivers ?? []) {
    r.anchor = point(r.node, r.anchor);
    if (r.navigationHeight !== undefined) r.navigationHeight += offset(r.node)[2];
    if (r.receiverSegment) r.receiverSegment = segment(r.node, r.receiverSegment);
  }
  result.doors.forEach(door);
  for (const lift of result.lifts ?? []) {
    lift.doors.forEach(door);
    if (lift.joins) lift.joins = line(lift.node, lift.joins);
  }
  for (const interior of result.interiors ?? []) {
    interior.doors.forEach(door);
    for (const join of interior.joins ?? []) join.point = point(interior.node, join.point);
  }
  for (const material of result.materials ?? [])
    material.polygon = line(material.node, material.polygon);
  for (const sound of result.sounds ?? [])
    if (sound.spatial) sound.spatial.polyline = line(sound.node, sound.spatial.polyline);
  for (const animation of result.animations ?? []) {
    animation.anchor = point(animation.node, animation.anchor);
    animation.displayPolyline = line(animation.node, animation.displayPolyline);
  }
  for (const light of result.lights ?? []) {
    light.polygon = line(light.node, light.polygon);
    if (light.receivers) light.receivers = line(light.node, light.receivers);
    if (light.receiverSegments)
      light.receiverSegments = light.receiverSegments.map((s) => segment(light.node, s));
    if (light.receiverPolylines)
      light.receiverPolylines = light.receiverPolylines.map((s) => line(light.node, s));
  }
  for (const mask of result.masks ?? []) {
    mask.anchor = point(mask.node, mask.anchor);
    mask.triangles = mask.triangles.map(([a, b, c]) => [
      point(mask.node, a),
      point(mask.node, b),
      point(mask.node, c),
    ]);
    if (mask.receiverSegment) mask.receiverSegment = segment(mask.node, mask.receiverSegment);
    if (mask.receiverPolyline) mask.receiverPolyline = line(mask.node, mask.receiverPolyline);
    if (mask.receiverPolylines)
      mask.receiverPolylines = mask.receiverPolylines.map((s) => line(mask.node, s));
    if (mask.characterBoundary) mask.characterBoundary = line(mask.node, mask.characterBoundary);
    if (mask.projectileBoundary) mask.projectileBoundary = line(mask.node, mask.projectileBoundary);
  }
  for (const zone of result.jumpZones ?? []) {
    zone.anchor = point(zone.node, zone.anchor);
    zone.polygon = line(zone.node, zone.polygon);
  }
  for (const pair of result.jumpPairs ?? [])
    for (const edge of pair.edges) {
      edge.a = point(pair.node, edge.a);
      edge.b = point(pair.node, edge.b);
    }
  for (const jump of result.jumpSegments ?? []) {
    if (jump.join) jump.join = point(jump.node, jump.join);
    jump.edge.a = point(jump.node, jump.edge.a);
    jump.edge.b = point(jump.node, jump.edge.b);
  }
  for (const transition of result.movementTransitions ?? []) {
    transition.waypoint = point(transition.node, transition.waypoint);
    if (transition.waypointAnchor)
      transition.waypointAnchor = point(transition.node, transition.waypointAnchor);
    if (transition.waypointReceiverSegment)
      transition.waypointReceiverSegment = segment(
        transition.node,
        transition.waypointReceiverSegment,
      );
    if (transition.join) transition.join.point = point(transition.node, transition.join.point);
    transition.applyPolygon = transition.applyPolygon.map((p) => xy(transition.node, p));
    transition.noApplyPolygon = transition.noApplyPolygon.map((p) => xy(transition.node, p));
    transition.initial.forEach(surface);
    transition.applied.forEach(surface);
  }
  return result;
}
