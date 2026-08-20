// Types for the hackable-datadir level JSON (`<Map>.rhp.json`, `<Mission>.rhm.json`)
// as written by `convert_datadir` in Hackable mode. Only the fields the editor
// and pipeline consume are typed; unknown fields pass through untouched.
//
// Coordinate system: world (x, y, z) projects to map pixels as (x, y - z).
// All polygon points are in map/world pixel units at 1:1 image resolution.

export type Point = [number, number];

export interface Polygon {
  points: Point[];
}

export interface ObstaclePoint {
  x: number;
  y: number;
  z_bottom: number;
  z_top: number;
}

export interface SightObstacle {
  points: ObstaclePoint[];
  projection_area: unknown;
  opaque: boolean;
  solid: boolean;
  mouse: boolean;
  show_shadow_polygon: boolean;
  default_material: number;
  material_indices: number[];
}

export interface Mask {
  layer: number;
  mask_type: number;
  character_polyline: Point[];
  projectile_polyline: Point[];
  box_top_left: Point;
  box_size: Point;
  /** RLE bytes — still opaque in converter output (todo: converter should emit PNG) */
  mask_data: number[];
  obstacle_indices: number[];
}

export interface MotionObstacle {
  polygon: Polygon;
  [k: string]: unknown;
}

export interface MotionArea {
  is_lift: boolean;
  state_id: number;
  polygon: Polygon;
  skeleton_segments: unknown[];
  flags: number;
  obstacles: MotionObstacle[];
}

export interface MotionData {
  /** layers[layer][area] */
  layers: MotionArea[][];
  /** pre-baked pathfinder graph — opaque, cannot be regenerated */
  graph_bytes: number[];
}

export interface ElementFxSprite {
  frame_profile_name: string;
  profile_name: string;
  position_x: number;
  position_y: number;
  elevation: number;
}

export interface ElementFx {
  sprite: ElementFxSprite;
  blit_type: number;
  active: boolean;
  force_display: boolean;
  display_polyline: Point[];
}

export interface Patch {
  element_fx: ElementFx;
  active: boolean;
  waypoint: Point;
  sector: number;
  layer: number;
  definitive: boolean;
  integrate_in_background: boolean;
  old_masks: number[];
  old_sight_obstacles: number[];
  new_masks: number[];
  new_sight_obstacles: number[];
  apply_sector: Polygon;
  no_apply_sector: Polygon;
  door_triggered: boolean;
  triggers_door: boolean;
  door_indices: number[];
  final_layer: number;
  [k: string]: unknown;
}

export interface MaterialSector {
  material: number;
  polygon: Polygon;
}

export interface LightSector {
  layer: number;
  polygon: Polygon;
  ambience: number;
}

export interface ElevationLine {
  point_a: Point;
  point_b: Point;
  right_obstacle_index: number;
  left_obstacle_index: number;
  layer: number;
}

export interface SoundSource {
  id: number;
  active: boolean;
  polyline: Point[];
  inner_distance: number;
  outer_distance: number;
  [k: string]: unknown;
}

export interface JumpZone {
  polygon: Polygon;
  sector: number;
  layer: number;
  helper_needed: boolean;
}

export interface JumpLine {
  point_a: Point;
  point_b: Point;
  jump_zone_index: number;
}

export interface JumpLinePair {
  line1: JumpLine;
  line2: JumpLine;
  jump_long: boolean;
}

export interface Lift {
  motion_area_index: number;
  lift_type: number;
  doors: unknown[];
  direction: number;
}

/** `<Map>.rhp.json` — per-map proto level */
export interface ProtoLevel {
  format: string;
  misc: { control_crc: number; forest_level: boolean; default_material: number };
  patches: Patch[];
  animations: ElementFx[];
  material_sectors: MaterialSector[];
  light_sectors: LightSector[];
  elevation_lines: ElevationLine[];
  masks: Mask[];
  sight_obstacles: SightObstacle[];
  sound_sources: SoundSource[];
  jump_zones: JumpZone[];
  jump_line_pairs: JumpLinePair[];
  lifts: Lift[];
  buildings: unknown[];
  motion_data: MotionData;
}

export interface MissionHeader {
  control_crc: number;
  ambiance: number;
  map_filename: string;
  mission_profile_id: number;
}

export interface BeamMe {
  position: Point;
  direction: number;
  layer: number;
  index: number;
  script: unknown;
  [k: string]: unknown;
}

export interface Soldier {
  position_x: number;
  position_y: number;
  direction: number;
  layer: number;
  profile_number: number;
  tower_guard: boolean;
  company_number: number;
  path_id: number;
  alert_path_id: number;
  [k: string]: unknown;
}

export interface Civilian {
  position_x: number;
  position_y: number;
  direction: number;
  layer: number;
  profile_number: number;
  path_id: number;
  [k: string]: unknown;
}

export interface HikingPath {
  waypoints: { position?: Point; [k: string]: unknown }[] | unknown;
}

/** `<Mission>.rhm.json` */
export interface Mission {
  format: string;
  header: MissionHeader;
  beam_mes: BeamMe[];
  soldiers: Soldier[];
  civilians: Civilian[];
  targets: { position_x: number; position_y: number; [k: string]: unknown }[];
  bonuses: { position_x: number; position_y: number; bonus_type: number; [k: string]: unknown }[];
  pcs_to_rescue: { position_x: number; position_y: number; [k: string]: unknown }[];
  scrolls: { position_x: number; position_y: number; [k: string]: unknown }[];
  hiking_paths: HikingPath[];
  script_objects: { points: unknown[]; lines: unknown[]; sectors: unknown[] };
  tactic_data: {
    reinforcement_points: unknown[];
    ambush_points: unknown[];
    seek_points: unknown[];
    archery_sectors: unknown[];
  };
  [k: string]: unknown;
}

export const AMBIANCE_DIRS = ["Day", "Fog", "Night", "Custom1"] as const;
export type AmbianceDir = (typeof AMBIANCE_DIRS)[number];
