import type { Mask, Point } from "../../shared/src/level.ts";
import type { Vec3 } from "../../shared/src/scene.ts";
import type { AssetOcclusionMask } from "../../shared/src/asset-gameplay.ts";
import { maskBoundaryPolyline, type MaskTriangle } from "../../shared/src/compile-mask-geometry.ts";
import { recoverMaskSurface } from "./recover-mask-surface.ts";

export interface MaskRecoveryDefinition {
  id: string;
  node: string;
  /** Reviewed world-space point on the receiving navigation surface. */
  anchor: Vec3;
  /** Authored finite world-space receiving reach, localized with the mask. */
  receiverSegment?: [Vec3, Vec3];
  /** Explicit owner surfaces in placed game coordinates. */
  surfaces: MaskTriangle[];
  /** Per-vertex heights from authoring evidence, not inferred from bitmap bounds. */
  characterHeights?: number[];
  projectileHeights?: number[];
  /** Source indices are resolved once to this owner's local part/volume IDs. */
  obstacles: ReadonlyMap<number, string>;
  localize: (point: Vec3) => Vec3;
}

/** Recover one complete rule definition after ownership and receiving geometry
 * have been established. Character points are projected; projectile points are
 * world XY. Their heights are supplied independently and never interchanged. */
export function recoverOcclusionMask(
  source: Mask,
  definition: MaskRecoveryDefinition,
): AssetOcclusionMask {
  const flags = source.mask_type;
  if (!Number.isInteger(flags) || flags <= 0 || flags > 23 || (flags & ~23) !== 0)
    throw new Error("Unsupported mask application flags");
  const obstacleRule = (flags & 16) !== 0;
  if (obstacleRule !== source.obstacle_indices.length > 0 || (obstacleRule && !(flags & 2)))
    throw new Error("Inconsistent mask obstacle rule");
  const local = (point: Vec3): Vec3 => {
    if (point.length !== 3 || !point.every(Number.isFinite))
      throw new Error("Invalid mask recovery point");
    const result = definition.localize([...point]);
    if (result.length !== 3 || !result.every(Number.isFinite))
      throw new Error("Invalid local mask recovery point");
    return [...result];
  };
  const boundary = (
    points: Point[] | null,
    heights: number[] | undefined,
    enabled: boolean,
    projected: boolean,
  ): Vec3[] | undefined => {
    if (!enabled) {
      if (points !== null || heights !== undefined) throw new Error("Unexpected mask boundary");
      return undefined;
    }
    if (!points || (!points.length && (projected || !obstacleRule)))
      throw new Error("Missing mask boundary");
    if (!points.length) {
      if (heights?.length) throw new Error("Unexpected mask boundary heights");
      return undefined;
    }
    if (!heights || heights.length !== points.length || !heights.every(Number.isFinite))
      throw new Error("Missing mask boundary elevation evidence");
    if (points.some((p) => p.length !== 2 || !p.every(Number.isInteger)))
      throw new Error("Invalid source mask boundary");
    if (JSON.stringify(maskBoundaryPolyline(points, false)) !== JSON.stringify(points))
      throw new Error("Source mask boundary is not ordered left to right");
    return points.map(([x, y], i) => local([x, projected ? y + heights[i]! : y, heights[i]!]));
  };
  const characterBoundary = boundary(
    source.character_polyline,
    definition.characterHeights,
    (flags & 1) !== 0,
    true,
  );
  const projectileBoundary = boundary(
    source.projectile_polyline,
    definition.projectileHeights,
    (flags & 2) !== 0,
    false,
  );
  const obstacles = source.obstacle_indices.map((index) => {
    const id = definition.obstacles.get(index);
    if (!id) throw new Error(`Mask obstacle ${index} has no local owner reference`);
    return id;
  });
  if (new Set(obstacles).size !== obstacles.length)
    throw new Error("Mask obstacles do not have distinct local references");
  return {
    id: definition.id,
    node: definition.node,
    anchor: local(definition.anchor),
    ...(definition.receiverSegment
      ? {
          receiverSegment: [
            local(definition.receiverSegment[0]),
            local(definition.receiverSegment[1]),
          ] as [Vec3, Vec3],
        }
      : {}),
    view: (flags & 4) !== 0,
    ...(characterBoundary ? { characterBoundary, characterBoundaryClosed: false } : {}),
    ...(projectileBoundary ? { projectileBoundary, projectileBoundaryClosed: false } : {}),
    obstacles,
    triangles: recoverMaskSurface(source, definition.surfaces, local),
  };
}
