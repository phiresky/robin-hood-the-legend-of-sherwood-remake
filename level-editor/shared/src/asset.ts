// Asset library schema. One directory per asset under `level-editor/library/`:
//   <id>/asset.json           — this descriptor
//   <id>/day.png              — RGBA cutout (mask-applied), asset-local coords
//   <id>/fog.png, night.png   — same region cut from other ambiance images, if present
//   <id>/mask.png             — 8-bit alpha mask, same size as cutouts
//
// Asset-local coordinates: pixel (0,0) of the cutout corresponds to world/map
// pixel `origin` in the source map. All carried-over geometry is stored in
// asset-local coordinates (world minus origin) so placing an asset at P just
// adds P back.

import type {
  ObstaclePoint,
  Point,
  Polygon,
  JumpZone,
  JumpLinePair,
  Lift,
  Mask,
} from "./level";

export type ScaleClass = "unique" | "variant" | "spline-segment" | "texture";

export interface AssetSource {
  /** map name as in the datadir, e.g. "Leicester" */
  map: string;
  /** ambiance the mask was authored on (always "Day" for now) */
  ambiance: string;
  /** bounding box of the cutout in source-map pixels: [x, y, w, h] */
  bbox: [number, number, number, number];
  /** how the mask was produced, for reproducibility */
  extraction: {
    tool: string;
    prompt?: string;
    points?: Point[];
    boxes?: [number, number, number, number][];
    score?: number;
  };
}

export interface AssetVolumes {
  sight_obstacles: { points: ObstaclePoint[]; opaque: boolean; solid: boolean }[];
}

export interface AssetMotion {
  /** obstacle polygons (unwalkable footprints) clipped from the source layers */
  obstacles: { layer: number; polygon: Polygon }[];
  /** walkable polygon fragments intersecting the region, if any */
  walkable: { layer: number; polygon: Polygon }[];
}

export interface AssetDescriptor {
  id: string;
  name: string;
  tags: string[];
  scale_class: ScaleClass;
  /** ids of sibling assets that are variants of the same thing */
  variant_group?: string;
  source: AssetSource;
  /** world-pixel position of cutout pixel (0,0) in the source map */
  origin: Point;
  /**
   * anchor point in asset-local pixels — the "ground contact" reference used
   * for placement and draw-order sorting (projected map Y of the base line)
   */
  anchor: Point;
  images: { day: string; fog?: string; night?: string; mask: string };
  volumes: AssetVolumes;
  motion: AssetMotion;
  /** carried-over extras, asset-local coords; all optional */
  doors?: unknown[];
  jump_zones?: JumpZone[];
  jump_line_pairs?: JumpLinePair[];
  lifts?: Lift[];
  material_sectors?: { material: number; polygon: Polygon }[];
  sound_sources?: unknown[];
  /** legacy occlusion masks intersecting the region (polylines only for now) */
  occlusion_masks?: Pick<Mask, "layer" | "character_polyline" | "projectile_polyline">[];
  /**
   * for spline-segment wall pieces: screen-space direction the art runs,
   * degrees in (-90, 90], 0 = horizontal, negative = ascending left→right
   */
  wall_direction_deg?: number;
  /** set for assets imported from .rhs.d sprite banks (animated FX / patches) */
  fx?: {
    /** sprite bank name, e.g. "Leifx" (Data/Animations/<Ambiance>/<bank>.rhs.d) */
    bank: string;
    profile: string;
    action: string;
    frame_count: number;
    /** original world draw position + elevation from the proto level */
    position: Point;
    elevation: number;
    hotspot: Point;
  };
  notes?: string;
}

export interface LibraryIndexEntry {
  id: string;
  name: string;
  tags: string[];
  scale_class: ScaleClass;
  source_map: string;
  bbox: [number, number, number, number];
}
