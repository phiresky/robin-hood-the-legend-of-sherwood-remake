import polygonClipping from "polygon-clipping";
import type { CompiledAssetGeometry, PhysicalStairNavigation } from "./asset-gameplay.ts";
import { heightPlane, planeHeight } from "./gameplay-plane.ts";
import type { Point } from "./level.ts";
import type { Vec3 } from "./scene.ts";
import { pointInGameplayPolygon } from "./navigation-anchor.ts";
import { physicalCollisionPieces } from "./physical-collision-pieces.ts";

export interface PhysicalStairInput {
  surfaces: { polygon: Vec3[]; holes: Vec3[][] }[];
  /** Collision pieces already bound to the emitted motion area's obstacle IDs. */
  obstacles: { motionObstacle: number; polygon: Vec3[] }[];
  doors: PhysicalStairNavigation["doors"];
}

export interface PhysicalStairAreaInput extends Omit<PhysicalStairInput, "obstacles"> {
  obstacles: { stateId: number; polygon: Vec3[] }[];
}

/** Allocate one authoritative motion identity for every physical collision piece. */
export function compilePhysicalStairArea(input: PhysicalStairAreaInput): {
  area: CompiledAssetGeometry["motion_data"]["layers"][number][number];
  navigation: PhysicalStairNavigation;
} {
  for (const obstacle of input.obstacles)
    if (
      !Number.isInteger(obstacle.stateId) ||
      obstacle.stateId < 0 ||
      obstacle.stateId > 0xffffffff
    )
      throw new Error("Physical stair collision needs a valid motion state word");
  const { navigation, holes } = compilePhysicalStair({
    ...input,
    obstacles: input.obstacles.map((obstacle, motionObstacle) => ({
      motionObstacle,
      polygon: obstacle.polygon,
    })),
  });
  const collision = [
    ...holes.map((polygon) => ({ stateId: 0, polygon })),
    ...navigation.obstacles.map((obstacle, index) => ({
      stateId: input.obstacles[index]!.stateId,
      polygon: obstacle.polygon,
    })),
  ].flatMap((obstacle) =>
    physicalCollisionPieces(obstacle.polygon).map((polygon) => ({ ...obstacle, polygon })),
  );
  if (collision.length > 65536)
    throw new Error("Physical stair exceeds motion obstacle identity capacity");
  navigation.obstacles = collision.map((obstacle, motion_obstacle) => ({
    motion_obstacle,
    polygon: obstacle.polygon,
  }));
  const project = (point: Point): Point => {
    const result: Point = [
      Math.round(point[0]),
      Math.round(point[1] - planeHeight(navigation.plane, point)),
    ];
    if (result.some((value) => !Number.isFinite(value) || value < -32768 || value > 32767))
      throw new Error("Physical stair projection exceeds the game coordinate range");
    return result;
  };
  return {
    navigation,
    area: {
      is_lift: true,
      state_id: 0,
      flags: 0,
      skeleton_segments: [],
      // A valid physical floor can project to a line. Preserve its ordered
      // vertices; simplifying that line would lose the physical surface identity.
      polygon: { points: navigation.boundary.map(project) },
      obstacles: collision.map((obstacle) => ({
        state_id: obstacle.stateId,
        polygon: { points: obstacle.polygon.map(project) },
      })),
    },
  };
}

/** Assemble a placed floor before projection/quantization can destroy its area. */
export function compilePhysicalStair(input: PhysicalStairInput): {
  navigation: PhysicalStairNavigation;
  /** These permanent holes still need motion obstacle IDs from the area compiler. */
  holes: Point[][];
} {
  const vertices = input.surfaces.flatMap((surface) => surface.polygon);
  if (vertices.some((point) => point.some((value) => !Number.isFinite(value))))
    throw new Error("Physical stair vertices must be finite");
  if (vertices.length < 3) throw new Error("Physical stair needs a floor");
  const plane = heightPlane(vertices);
  const groundRing = (points: Vec3[], label: string): Point[] => {
    if (
      points.length < 3 ||
      points.some(
        (point) =>
          point.some((value) => !Number.isFinite(value)) ||
          Math.abs(planeHeight(plane, [point[0], point[1]]) - point[2]) > 1e-4,
      )
    )
      throw new Error(`${label} must be a finite polygon on the stair floor`);
    const ring = points.map(([x, y]): Point => [x, y]);
    const area = ring.reduce((sum, point, i) => {
      const next = ring[(i + 1) % ring.length]!;
      return sum + point[0] * next[1] - point[1] * next[0];
    }, 0);
    if (Math.abs(area) <= 1e-8) throw new Error(`${label} has no physical area`);
    if (area < 0) ring.reverse();
    return ring;
  };
  const floors = input.surfaces.map((surface) => [
    groundRing(surface.polygon, "Physical stair surface"),
    ...surface.holes.map((hole) => groundRing(hole, "Physical stair hole")),
  ]);
  const merged = polygonClipping.union(floors[0]!, ...floors.slice(1));
  if (merged.length !== 1) throw new Error("Physical stair surfaces must form one connected floor");
  const open = (ring: Point[]): Point[] => {
    const first = ring[0]!,
      last = ring.at(-1)!;
    return first[0] === last[0] && first[1] === last[1] ? ring.slice(0, -1) : ring;
  };
  const boundary = open(merged[0]![0]!);
  const holes = merged[0]!
    .slice(1)
    .filter((ring) => !isPhysicalClippingSliver(ring))
    .map(open);
  const obstacles = input.obstacles.map((obstacle) => {
    if (
      !Number.isInteger(obstacle.motionObstacle) ||
      obstacle.motionObstacle < 0 ||
      obstacle.motionObstacle > 65535
    )
      throw new Error("Physical stair obstacle needs a valid motion-area identity");
    return {
      motion_obstacle: obstacle.motionObstacle,
      polygon: groundRing(obstacle.polygon, "Physical stair obstacle"),
    };
  });
  const contains = (point: Vec3, ring: Point[]) =>
    pointInGameplayPolygon([point[0], point[1]], ring, true);
  const doors = input.doors.map((door, index): PhysicalStairNavigation["doors"][number] => {
    for (const point of [door.inside, door.middle, door.outside])
      if (point.some((value) => !Number.isFinite(value)))
        throw new Error("Physical stair door coordinates must be finite");
    for (const anchor of ["inside", "middle"] as const) {
      const point = door[anchor];
      const heightError = point[2] - planeHeight(plane, [point[0], point[1]]);
      const label = `door ${index} ${anchor} at ${JSON.stringify(point)}`;
      if (Math.abs(heightError) > 1e-4)
        throw new Error(
          `Physical stair anchors must lie on the floor: ${label}, height difference ${heightError}`,
        );
      if (!contains(point, boundary) || holes.some((hole) => contains(point, hole)))
        throw new Error(`Physical stair anchors must have floor support: ${label}`);
    }
    return { inside: [...door.inside], middle: [...door.middle], outside: [...door.outside] };
  });
  return { navigation: { plane, boundary, obstacles, doors }, holes };
}

/** Only for clipping results: authored contours must pass normal validation. */
export function isPhysicalClippingSliver(ring: Point[]): boolean {
  if (ring.length < 3) return true;
  let start = ring[0]!,
    end = start,
    longest = 0;
  let scale = 1;
  for (const [i, point] of ring.entries()) {
    scale = Math.max(scale, Math.abs(point[0]), Math.abs(point[1]));
    const next = ring[(i + 1) % ring.length]!;
    const length = Math.hypot(next[0] - point[0], next[1] - point[1]);
    if (length > longest) {
      start = point;
      end = next;
      longest = length;
    }
  }
  if (longest === 0) return true;
  const tolerance = scale * Number.EPSILON * 8;
  return ring.every(
    (point) =>
      Math.abs(
        (end[0] - start[0]) * (point[1] - start[1]) - (end[1] - start[1]) * (point[0] - start[0]),
      ) <=
      tolerance * longest,
  );
}
