import type { LightSector, MaterialSector, Point, SightObstacle, SoundSource } from "./level.ts";
import type { ProjectionAssetDescriptor } from "./projection-assets.ts";
import { safeLibraryPath } from "./projection-assets.ts";
import type { Vec3 } from "./scene.ts";

/** All coordinates belong to the named mesh part's local game frame. No level indices. */
export interface AssetWalkableSurface {
  id: string;
  node: string;
  polygon: Point[];
  /** Constant height or one height per polygon vertex; the surface must be planar. */
  height: number | number[];
  /** Static blockers and clearances: match collision on this local navigation plane while projecting the footprint at its physical height. */
  navigationHeight?: number;
  /** Transition blockers may follow terrain within these finite vertical offsets from their plane. */
  terrainReach?: { below: number; above: number };
  /** Generate reusable long-jump ledges and landing bands from this surface after placement. */
  jump?: NonNullable<AssetJumpSegment["attachment"]> & {
    /** Selected polygon edge indices; omit to consider every outer edge. */
    edges?: number[];
    /** Minimum distance from the boundary to the takeoff line; walking clearance may increase it. */
    inset: number;
    /** Walkable depth behind the takeoff line required for a receiving band. */
    landingDepth: number;
    /** Maximum horizontal outer-edge endpoint adjustment onto a level contour, before inset. */
    maxLevelAdjustment?: number;
  };
  /** Retain fractional boundaries through movement assembly; only the final regions snap to the grid. */
  preserveMovementPrecision?: boolean;
  /** Keep this ordinary area's outer contour separate from crossing movement obstacles. */
  preserveMovementBoundary?: boolean;
  /** Holes lie on the same plane, in the same local XY frame. */
  holes?: Point[][];
  /** Shared assembly labels for preserved hole contours, aligned with holes. */
  holeContours?: string[];
  /** Shared assembly label for fragments of one movement exclusion.
   * Transition labels are scoped to their placement, transition and initial/applied state. */
  movementContour?: string;
  /** Asset-local navigation region, optionally spanning height planes; distinct regions never merge. */
  navigationRegion?: string;
  /** Local 3D outer-edge sockets joining navigation regions of separately placed assets.
   * Endpoints coincide unless both owners opt into a minimum shared span; unmatched edges stay separate. */
  navigationJoins?: import("./assemble-navigation-joins.ts").NavigationJoin[];
  /** Explicit maximum height step at coincident projected sockets. Both assets must allow it. */
  navigationJoinHeightTolerance?: number;
  /** Opt in to partial edge connections with this minimum shared span in map units.
   * Both assets must opt in; disjoint neighbors may use separate spans of one socket. */
  navigationJoinMinimumOverlap?: number;
  /** Local part or gameplay volume supplying receiving geometry, physical flags and materials.
   * Replaces the generated thin receiver; activation uses the volume's sight-state links. */
  projectionVolume?: string;
  /** Receiving-surface material and ordered asset-local material-region references. */
  projectionMaterials?: {
    defaultMaterial: number;
    regions: string[];
    /** Ordered local plane anchors, retained through clipping for native height arithmetic. */
    planePoints?: [[number, number, number], [number, number, number], [number, number, number]];
    /** Local bounding height for overlap priority; equal heights use surface order within the asset. */
    priorityHeight?: number;
    /** Higher values win equal-height overlaps across independent placements. */
    priority?: number;
    /** Local receiving footprint, including blocked portions omitted from walking contours. */
    footprint?: [number, number, number][];
  };
}
export interface AssetDoor {
  id: string;
  node: string;
  polygon: Point[];
  /** Local [x, y, elevation]; endpoints resolve against assembled walkable surfaces. */
  outside: [number, number, number];
  inside: [number, number, number];
  middle: [number, number, number];
  /** Optional unblocked local points selecting receiving areas independently of door coordinates. */
  outsideAnchor?: [number, number, number];
  insideAnchor?: [number, number, number];
  /** Entrance or ordinary passage approach follows one unblocked surface within this local segment. */
  outsideReceiverSegment?: [[number, number, number], [number, number, number]];
  /** Ordinary passage destination follows its own finite receiver; virtual rooms and lifts remain authored. */
  insideReceiverSegment?: [[number, number, number], [number, number, number]];
  type: number;
  locked: boolean;
  unlockable: boolean;
  active?: boolean;
  /** An unrestricted, non-clickable passage may disappear when its areas join. */
  allowContinuous?: boolean;
  lockedVillains?: boolean;
  lockedCivilians?: boolean;
  /** Alternate lock rules; activation still requires an authored state transition. */
  afterTransition?: {
    locked: boolean;
    unlockable: boolean;
    lockedVillains: boolean;
    lockedCivilians: boolean;
  };
}
export interface AssetLift {
  id: string;
  node: string;
  /** Asset-local surface ID; no runtime sector or layer number. */
  surface: string;
  /** Local 3D sockets joining another lift segment after placement. Every socket must match. */
  joins?: [number, number, number][];
  type: 1 | 2 | 3;
  /** Local ground-plane direction; transformed with the owning part. */
  direction: Point;
  doors: AssetDoor[];
}
export interface AssetInterior {
  id: string;
  node: string;
  /** Entrances share one virtual interior; occupants are authored separately. */
  doors: AssetDoor[];
  /** Optional passage sockets; coincident opposing sockets connect rooms after placement. */
  joins?: { point: [number, number, number]; direction: Point }[];
}
export interface AssetMaterialRegion {
  id: string;
  node: string;
  /** Local 3D vertices, projected after placement. */
  polygon: [number, number, number][];
  /** Material codes 0–8; 9 selects the map default. */
  material: number;
  /** Register in the ground-layer lookup, independently of obstacle links. */
  ground: boolean;
  /** Owning asset's part nodes whose impact material lookup uses this region. */
  obstacles: string[];
}
export interface AssetGameplay {
  /** Local game-space elevation to rest on terrain when inserting this asset. */
  placementGroundHeight?: number;
  version: 1;
  /** Model calibration for deforming gameplay with a spline; authored from the asset model. */
  spline?: {
    modelSha256?: string;
    bounds: { min: import("./scene.ts").Vec3; max: import("./scene.ts").Vec3 };
    /** Column-major local-part to asset-scene transforms, including gameplay-only frames. */
    frames: Record<string, number[]>;
    /** Measurements from the asset mesh for rotated or straightened source sections. */
    deformations?: import("./wall-section-profile.ts").WallSourceCalibration[];
  };
  /** Publish usable definitions while retaining known gaps in every compilation report.
   * Absence does not certify parity; it only means no draft issues were recorded. */
  draft?: { issues: string[] };
  /** Reuse part obstacles or disable them; explicit gameplay volumes remain independent. */
  collision: "parts" | "none";
  /** Query precedence for local physical part/volume IDs; lower values run first. */
  sightOrder?: Record<string, number>;
  /** Gameplay volumes attached to an existing frame; no rendered mesh is required. */
  volumes?: {
    id: string;
    node: string;
    /** Required upright space beneath this permanent movement solid, in world
     * game-height units. Defaults to zero; sight/projectile geometry is unchanged. */
    movementHeadroom?: number;
    shape: Omit<SightObstacle, "projection_area" | "material_indices">;
  }[];
  /** Authored contours replace implicit movement derivation; movementSolids can select additional solids. */
  movementBlockers?: AssetWalkableSurface[];
  /** Explicit part/volume IDs supplying permanent movement solids, independently of sight states. */
  movementSolids?: string[];
  /** Plane-local openings in this asset's derived movement collision, never in other assets. */
  movementClearances?: AssetWalkableSurface[];
  surfaces: AssetWalkableSurface[];
  /** Physical receivers sharing existing navigation without adding a walking boundary. */
  projectionReceivers?: {
    id: string;
    node: string;
    volume: string;
    /** Local navigation anchor, or physical anchor when navigationHeight supplies the walking plane. */
    anchor: [number, number, number];
    /** Project the physical anchor onto this local navigation plane after placement. */
    navigationHeight?: number;
    /** Optional finite local reach selecting navigation beneath the physical receiver. */
    receiverSegment?: [[number, number, number], [number, number, number]];
  }[];
  doors: AssetDoor[];
  lifts?: AssetLift[];
  interiors?: AssetInterior[];
  materials?: AssetMaterialRegion[];
  /** Map-wide defaults supplied by a terrain asset. Ambience is mission-owned. */
  environment?: { forest: boolean; defaultMaterial: number };
  sounds?: AssetSoundSource[];
  /** Billboard scenery; the local anchor moves with its owner while artwork faces the camera. */
  animations?: AssetSceneryAnimation[];
  lights?: AssetLightRegion[];
  masks?: AssetOcclusionMask[];
  /** Parts whose complete sprite occlusion is authored by typed masks, including states.
   * Keep their color geometry but omit their contribution from the static depth bake. */
  maskOcclusionNodes?: string[];
  jumpZones?: AssetJumpZone[];
  jumpPairs?: AssetJumpPair[];
  jumpSegments?: AssetJumpSegment[];
  /** Independent navigation, sight, mask and door state links; visual resources are separate. */
  movementTransitions?: AssetMovementTransition[];
}
export interface AssetOcclusionMask {
  id: string;
  node: string;
  /** Explicit local 3D coverage, including cutouts between triangles. */
  triangles: import("./compile-mask-geometry.ts").MaskTriangle[];
  /** Match one-sided mesh faces after placement; omitted masks remain two-sided. */
  cullBackfaces?: boolean;
  /** Local point on the receiving navigation surface; may lie inside a blocker. */
  anchor: [number, number, number];
  /** Optional finite local segment selecting the receiving layer instead of the exact anchor height. */
  receiverSegment?: [[number, number, number], [number, number, number]];
  /** Bend-preserving receiving probe; mutually exclusive with receiverSegment. */
  receiverPolyline?: [number, number, number][];
  /** Disconnected probes; all intersections must select the same receiving layer. */
  receiverPolylines?: [number, number, number][][];
  view: boolean;
  /** Local boundary; its projected front envelope controls character masking. */
  characterBoundary?: [number, number, number][];
  /** Defaults to true. False preserves an authored open polyline without a closing edge. */
  characterBoundaryClosed?: boolean;
  /** Local boundary; its world XY front envelope controls projectile masking. */
  projectileBoundary?: [number, number, number][];
  /** Defaults to true, independently of the character boundary. */
  projectileBoundaryClosed?: boolean;
  /** Local part/volume IDs used for the projectile/flying-human altitude test. */
  obstacles: string[];
}
export interface AssetLightRegion {
  id: string;
  node: string;
  /** Local contour group sharing receiver coverage after subdivision. */
  receiverGroup?: string;
  /** Planar local 3D contour; defaults to receivers on the same plane. */
  polygon: [number, number, number][];
  /** Optional local anchors selecting receiving layers independently of the contour plane. */
  receivers?: [number, number, number][];
  /** Finite local segments selecting one receiving surface after placement, including slopes. */
  receiverSegments?: [[number, number, number], [number, number, number]][];
  /** Subdivided receiving probes, preserving bends through spline deformation. */
  receiverPolylines?: [number, number, number][][];
  /** Mission ambience bit mask controlling this region, not a mission selection. */
  ambiences: number;
}
export interface AssetJumpZone {
  id: string;
  node: string;
  polygon: [number, number, number][];
  /** An unblocked point on the receiving movement surface. */
  anchor: [number, number, number];
  helperNeeded: boolean;
}
export interface AssetJumpSegment {
  id: string;
  node: string;
  long: boolean;
  /** Legacy exact socket. Geometric attachment rules do not require a socket. */
  join?: [number, number, number];
  /**
   * Connection limits in map units after placement; no neighbour identity.
   * The destination lies along the map-plane normal (-dy, dx) of a → b.
   * Both edges must opt in, face each other and agree on a usable shared span.
   */
  attachment?: {
    maxGap: number;
    maxRise: number;
    maxDrop: number;
    minOverlap: number;
    /** Optional upright body envelope in world units; omitted means foot-trajectory checks only. */
    clearance?: { radius: number; height: number };
  };
  edge: AssetJumpPair["edges"][number];
}
export interface AssetJumpPair {
  id: string;
  node: string;
  long: boolean;
  /** Each edge names its home zone; destination links are rebuilt during compilation. */
  edges: [
    { zone: string; a: [number, number, number]; b: [number, number, number] },
    { zone: string; a: [number, number, number]; b: [number, number, number] },
  ];
}
export interface AssetMovementTransition {
  id: string;
  /** Asset-local model appearance IDs controlled by this gameplay transition. */
  appearances?: string[];
  /** Parts share a switch only when matching asset-local anchors meet after placement. */
  join?: { key: string; point: [number, number, number] };
  node: string;
  waypoint: [number, number, number];
  /** Local receiving-area anchor when the reference point lies outside its linked surface. */
  waypointAnchor?: [number, number, number];
  /** Bind the state control to nearby terrain, including positions inside its closed blocker. */
  waypointReceiverSegment?: [[number, number, number], [number, number, number]];
  active: boolean;
  definitive: boolean;
  initial: AssetWalkableSurface[];
  applied: AssetWalkableSurface[];
  /** Local part/volume IDs enabled before and after the transition, respectively. */
  initialSight?: string[];
  appliedSight?: string[];
  initialMasks?: string[];
  appliedMasks?: string[];
  /** Local ordinary/interior door IDs. Lift traversal doors cannot bind map patches. */
  doorLinks?: { mode: "trigger-transition" | "swap-rights"; ids: string[] };
  /** Trigger contours at the waypoint's local elevation; empty means externally activated. */
  applyPolygon: Point[];
  noApplyPolygon: Point[];
}
export interface AssetSoundSource {
  id: string;
  node: string;
  /** Shared sound-bank sample identity, not a level source index. */
  sample: number;
  kind: 0 | 1 | 2 | 3;
  active: boolean;
  delay?: [number, number, number];
  /** Acoustic altitude category, independent of geometric elevation. */
  altitude: 0 | 1 | 2 | 3;
  ambiences: number;
  /** Omitted for a global emitter. Distances use game units, volumes use percent. */
  spatial?: {
    polyline: [number, number, number][];
    innerDistance: number;
    outerDistance: number;
    innerVolume: number;
    outerVolume: number;
    noiseCoveringDistance: number;
  };
}
export type GameplayAssetDescriptor = ProjectionAssetDescriptor & { gameplay?: AssetGameplay };

export interface AssetSceneryAnimation {
  id: string;
  node: string;
  anchor: [number, number, number];
  /** Sprite basename, with optional .rhs suffix; resources come from a mod or shared bank. */
  file: string;
  /** Library-root-relative .rhs.d folder; every file must have a descriptor resource pin. */
  resourceDirectory?: string;
  profile: string;
  /** Authored sprite center, used to convert the placed anchor into the runtime top-left. */
  center: Point;
  active: boolean;
  forceDisplay: boolean;
  shadow: boolean;
  displayPolyline: [number, number, number][];
}

/** A generated interchange schema; indices are assigned afresh on each compilation. */
export interface CompiledAssetGeometry {
  /** Authoring diagnostics, also surfaced in the export summary. */
  warnings?: string[];
  motion_data: {
    layers: {
      is_lift: boolean;
      state_id: number;
      polygon: { points: Point[] };
      precise_polygon?: Point[];
      skeleton_segments: never[];
      flags: number;
      obstacles: { state_id: number; polygon: { points: Point[] }; precise_polygon?: Point[] }[];
    }[][];
    graph_bytes: never[];
  };
  sight_obstacles: SightObstacle[];
  /** Baked typed masks; obstacle references use this compilation's sight array. */
  masks?: import("./level.ts").Mask[];
  material_sectors?: MaterialSector[];
  sight_material_indices?: number[];
  map_settings?: { forest_level: boolean; default_material: number };
  sound_sources?: SoundSource[];
  animations?: {
    sprite: {
      frame_profile_name: string;
      profile_name: string;
      position_x: number;
      position_y: number;
      elevation: number;
    };
    blit_type: number;
    active: boolean;
    force_display: boolean;
    display_polyline: Point[];
  }[];
  light_sectors?: LightSector[];
  jump_zones?: {
    polygon: { points: Point[] };
    sector: number;
    layer: number;
    helper_needed: boolean;
  }[];
  jump_line_pairs?: {
    line1: {
      point_a: [number, number, number];
      point_b: [number, number, number];
      jump_zone_index: number;
    };
    line2: {
      point_a: [number, number, number];
      point_b: [number, number, number];
      jump_zone_index: number;
    };
    jump_long: boolean;
  }[];
  movement_transitions?: {
    id: string;
    /** Placed member IDs sharing this switch, excluding its canonical ID. */
    aliases?: string[];
    has_appearance?: boolean;
    waypoint: Point;
    sector: number;
    layer: number;
    active: boolean;
    definitive: boolean;
    apply_polygon: { points: Point[] };
    no_apply_polygon: { points: Point[] };
    motion_changes: { layer: number; sector: number; changing_obstacle: number }[];
    initial_sight?: number[];
    applied_sight?: number[];
    /** Indices into this compilation's mask array, not native per-layer indices. */
    initial_masks?: number[];
    applied_masks?: number[];
    door_links?: { mode: "trigger-transition" | "swap-rights"; indices: number[] };
  }[];
  doors: {
    world_endpoints?: { inside: Vec3; middle: Vec3; outside: Vec3 };
    door_type: number;
    active: boolean;
    locked_pc: boolean;
    unlockable: boolean;
    locked_npc_villain: boolean;
    locked_npc_civilian: boolean;
    locked_pc_after_patch: boolean;
    unlockable_after_patch: boolean;
    locked_npc_villain_after_patch: boolean;
    locked_npc_civilian_after_patch: boolean;
    door_sector: { points: Point[] };
    point_out: Point;
    sector_out: number;
    layer_out: number;
    point_mid: Point;
    point_in: Point;
    sector_in: number;
    layer_in: number;
  }[];
  buildings?: { Building: { doors: CompiledAssetGeometry["doors"] } }[];
  lifts?: {
    motion_area_index: number;
    lift_type: number;
    direction: number;
    endpoint_doors?: [number, number];
    physical_navigation?: PhysicalStairNavigation;
    doors: CompiledAssetGeometry["doors"];
  }[];
}

/** Placed physical floors retain navigation independently of screen projection. */
export interface PhysicalStairNavigation {
  plane: [number, number, number];
  /** When present, retain each connected flight's own physical height plane. */
  floor_patches?: { plane: [number, number, number]; boundary: Point[] }[];
  boundary: Point[];
  obstacles: { motion_obstacle: number; polygon: Point[] }[];
  doors: { inside: Vec3; middle: Vec3; outside: Vec3 }[];
}

export function validateAssetGameplay(
  value: unknown,
  descriptor: ProjectionAssetDescriptor,
): asserts value is AssetGameplay {
  const fail = (message: string): never => {
    throw new Error(`Asset ${descriptor.id}: ${message}`);
  };
  if (!value || typeof value !== "object") fail("missing gameplay definition");
  const data = value as AssetGameplay;
  if (data.placementGroundHeight !== undefined && !Number.isFinite(data.placementGroundHeight))
    fail("invalid placement ground height");
  const disabledParts = new Set(
    descriptor.parts.filter((part) => part.collision === "none").map((part) => part.node),
  );
  if (data.version !== 1 || !["parts", "none"].includes(data.collision))
    fail("invalid gameplay version or collision mode");
  if (
    data.draft !== undefined &&
    (!data.draft ||
      typeof data.draft !== "object" ||
      Array.isArray(data.draft) ||
      !Array.isArray(data.draft.issues) ||
      data.draft.issues.length === 0 ||
      data.draft.issues.some((issue) => typeof issue !== "string" || !issue.trim()) ||
      new Set(data.draft.issues).size !== data.draft.issues.length)
  )
    fail("invalid gameplay draft issues");
  if (data.sightOrder !== undefined) {
    if (!data.sightOrder || typeof data.sightOrder !== "object" || Array.isArray(data.sightOrder))
      fail("invalid sight query order");
    for (const [id, order] of Object.entries(data.sightOrder))
      if (
        !Number.isSafeInteger(order) ||
        order < 0 ||
        disabledParts.has(id) ||
        !(
          data.volumes?.some((v) => v.id === id) ||
          (data.collision === "parts" &&
            descriptor.parts.some(
              (p) => p.node === id && p.obstacle_local_game && p.mission_profile === undefined,
            ))
        )
      )
        fail(`invalid sight query order for ${id}`);
  }
  if (
    data.environment !== undefined &&
    (descriptor.editor_usage !== "map-background" ||
      !data.environment ||
      typeof data.environment.forest !== "boolean" ||
      !Number.isInteger(data.environment.defaultMaterial) ||
      data.environment.defaultMaterial < 0 ||
      data.environment.defaultMaterial > 8)
  )
    fail("invalid terrain environment defaults");
  const point = (p: unknown, length: number) =>
    Array.isArray(p) &&
    p.length === length &&
    p.every((v) => typeof v === "number" && Number.isFinite(v));
  const nodes = new Set(descriptor.parts.map((part) => part.node));
  if (data.spline !== undefined) {
    const calibration = data.spline;
    if (
      !calibration ||
      !calibration.bounds ||
      !point(calibration.bounds.min, 3) ||
      !point(calibration.bounds.max, 3) ||
      calibration.bounds.min.some((n, i) => n >= calibration.bounds.max[i]!) ||
      !calibration.frames ||
      typeof calibration.frames !== "object" ||
      Array.isArray(calibration.frames)
    )
      fail("invalid spline model calibration");
    if (calibration.modelSha256 !== undefined && !/^[0-9a-f]{64}$/.test(calibration.modelSha256))
      fail("invalid spline model hash");
    if (calibration.deformations !== undefined && !Array.isArray(calibration.deformations))
      fail("invalid spline deformation calibrations");
    for (const deformation of calibration.deformations ?? []) {
      if (
        !deformation ||
        !["x", "y"].includes(deformation.axis) ||
        !Number.isFinite(deformation.sourceAngle) ||
        !Number.isFinite(deformation.sourceStart) ||
        !Number.isFinite(deformation.sourceEnd) ||
        deformation.sourceStart < 0 ||
        deformation.sourceEnd > 1 ||
        deformation.sourceStart >= deformation.sourceEnd ||
        typeof deformation.sourceStraight !== "boolean" ||
        !deformation.bounds ||
        !point(deformation.bounds.min, 3) ||
        !point(deformation.bounds.max, 3) ||
        deformation.bounds.min.some((n, i) => n >= deformation.bounds.max[i]!)
      )
        fail("invalid spline source deformation");
      const profile = deformation.profile;
      const axis = deformation.axis === "y" ? 1 : 0;
      const min = deformation.bounds.min[axis],
        span = deformation.bounds.max[axis] - min;
      if (
        (!deformation.sourceStraight && !profile) ||
        (profile &&
          (!Number.isFinite(profile.start) ||
            !Number.isFinite(profile.end) ||
            Math.abs(profile.start - (min + span * deformation.sourceStart)) > 1e-6 ||
            Math.abs(profile.end - (min + span * deformation.sourceEnd)) > 1e-6 ||
            !Array.isArray(profile.sections) ||
            profile.sections.length !== 65 ||
            profile.sections.some(
              (s) =>
                !s || !Number.isFinite(s.center) || !Number.isFinite(s.width) || s.width <= 0.001,
            )))
      )
        fail("invalid spline cross-section profile");
    }
    for (const [node, matrix] of Object.entries(calibration.frames))
      if (
        !nodes.has(node) ||
        !point(matrix, 16) ||
        matrix[3] !== 0 ||
        matrix[7] !== 0 ||
        matrix[11] !== 0 ||
        matrix[15] !== 1
      )
        fail(`invalid spline frame ${node}`);
  }
  if (
    data.maskOcclusionNodes !== undefined &&
    (!Array.isArray(data.maskOcclusionNodes) ||
      !data.masks?.length ||
      new Set(data.maskOcclusionNodes).size !== data.maskOcclusionNodes.length ||
      data.maskOcclusionNodes.some((node) => !nodes.has(node)))
  )
    fail("mask occlusion nodes require typed masks and unique existing part frames");
  const ids = new Set<string>();
  const feature = (f: { id: string; node: string }) => {
    if (!f || typeof f.id !== "string" || !f.id || ids.has(f.id))
      fail("gameplay IDs must be nonempty and unique");
    ids.add(f.id);
    if (!nodes.has(f.node) && !(descriptor.editor_usage === "map-background" && f.node === "$root"))
      fail(`unknown gameplay node ${f.node}`);
  };
  const polygon = (points: unknown) => {
    if (!Array.isArray(points) || points.length < 3 || !points.every((p) => point(p, 2)))
      fail("invalid gameplay polygon");
  };
  if (![data.surfaces, data.doors].every(Array.isArray))
    fail("surfaces and doors must be explicitly declared");
  const legacySpawns = (value as { spawns?: unknown }).spawns;
  if (legacySpawns !== undefined && (!Array.isArray(legacySpawns) || legacySpawns.length))
    fail("Player spawns belong to missions, not map assets");
  if (data.movementTransitions !== undefined && !Array.isArray(data.movementTransitions))
    fail("invalid movement transitions");
  const changingSight = new Set<string>();
  if (data.masks !== undefined && !Array.isArray(data.masks)) fail("invalid masks");
  const maskIds = new Set<string>();
  for (const mask of data.masks ?? []) {
    feature(mask);
    if (
      mask.receiverSegment !== undefined &&
      (!Array.isArray(mask.receiverSegment) ||
        mask.receiverSegment.length !== 2 ||
        !mask.receiverSegment.every((p) => point(p, 3)) ||
        !mask.receiverSegment[0].some((v, i) => v !== mask.receiverSegment![1][i]))
    )
      fail(`invalid mask receiving segment ${mask.id}`);
    if (
      mask.receiverPolyline !== undefined &&
      (mask.receiverSegment !== undefined ||
        !Array.isArray(mask.receiverPolyline) ||
        mask.receiverPolyline.length < 2 ||
        !mask.receiverPolyline.every(
          (p, i, line) => point(p, 3) && (!i || p.some((v, axis) => v !== line[i - 1]![axis])),
        ))
    )
      fail(`invalid mask receiving polyline ${mask.id}`);
    if (
      mask.receiverPolylines !== undefined &&
      (mask.receiverSegment !== undefined ||
        mask.receiverPolyline !== undefined ||
        !Array.isArray(mask.receiverPolylines) ||
        !mask.receiverPolylines.length ||
        !mask.receiverPolylines.every(
          (line) =>
            Array.isArray(line) &&
            line.length >= 2 &&
            line.every(
              (p, i) => point(p, 3) && (!i || p.some((v, axis) => v !== line[i - 1]![axis])),
            ),
        ))
    )
      fail(`invalid mask receiving polylines ${mask.id}`);
    if (
      !point(mask.anchor, 3) ||
      typeof mask.view !== "boolean" ||
      (mask.cullBackfaces !== undefined && typeof mask.cullBackfaces !== "boolean") ||
      !Array.isArray(mask.triangles) ||
      !mask.triangles.length ||
      !mask.triangles.every(
        (triangle) =>
          Array.isArray(triangle) && triangle.length === 3 && triangle.every((p) => point(p, 3)),
      ) ||
      !Array.isArray(mask.obstacles) ||
      new Set(mask.obstacles).size !== mask.obstacles.length
    )
      fail(`invalid mask ${mask.id}`);
    for (const [boundary, closed] of [
      [mask.characterBoundary, mask.characterBoundaryClosed],
      [mask.projectileBoundary, mask.projectileBoundaryClosed],
    ] as const) {
      if (closed !== undefined && (typeof closed !== "boolean" || boundary === undefined))
        fail(`invalid mask boundary closure ${mask.id}`);
      if (
        boundary !== undefined &&
        (!Array.isArray(boundary) ||
          boundary.length < (closed === false ? 2 : 3) ||
          !boundary.every((p) => point(p, 3)))
      )
        fail(`invalid mask boundary ${mask.id}`);
    }
    for (const ref of mask.obstacles)
      if (
        typeof ref !== "string" ||
        disabledParts.has(ref) ||
        !(
          data.volumes?.some((volume) => volume.id === ref) ||
          (data.collision === "parts" &&
            descriptor.parts.some((part) => part.node === ref && part.obstacle_local_game))
        )
      )
        fail(`mask ${mask.id} references missing obstacle ${ref}`);
    if (!mask.view && !mask.characterBoundary && !mask.projectileBoundary && !mask.obstacles.length)
      fail(`mask ${mask.id} has no application rule`);
    maskIds.add(mask.id);
  }
  const changingMasks = new Set<string>();
  const changingAppearances = new Set<string>();
  const triggeringDoors = new Set<string>();
  for (const transition of data.movementTransitions ?? []) {
    feature(transition);
    if (
      transition.join !== undefined &&
      (!transition.join ||
        typeof transition.join.key !== "string" ||
        !transition.join.key.trim() ||
        !point(transition.join.point, 3))
    )
      fail("invalid transition join anchor");
    if (transition.appearances !== undefined) {
      if (!Array.isArray(transition.appearances) || !transition.appearances.length)
        fail("invalid transition appearance bindings");
      for (const appearance of transition.appearances) {
        if (typeof appearance !== "string" || !appearance || changingAppearances.has(appearance))
          fail("invalid or multiply controlled transition appearance");
        changingAppearances.add(appearance);
      }
    }
    if (
      !point(transition.waypoint, 3) ||
      (transition.waypointAnchor !== undefined && !point(transition.waypointAnchor, 3)) ||
      (transition.waypointReceiverSegment !== undefined &&
        (transition.waypointAnchor !== undefined ||
          !Array.isArray(transition.waypointReceiverSegment) ||
          transition.waypointReceiverSegment.length !== 2 ||
          !transition.waypointReceiverSegment.every((p) => point(p, 3)) ||
          transition.waypointReceiverSegment[0].every(
            (n, i) => n === transition.waypointReceiverSegment![1][i],
          ))) ||
      typeof transition.active !== "boolean" ||
      typeof transition.definitive !== "boolean" ||
      !Array.isArray(transition.initial) ||
      !Array.isArray(transition.applied) ||
      (!transition.initial.length &&
        !transition.applied.length &&
        !transition.initialSight?.length &&
        !transition.appliedSight?.length &&
        !transition.initialMasks?.length &&
        !transition.appliedMasks?.length &&
        !transition.appearances?.length &&
        !transition.doorLinks)
    )
      fail(`invalid movement transition ${transition.id}`);
    for (const refs of [transition.initialMasks, transition.appliedMasks]) {
      if (refs === undefined) continue;
      if (!Array.isArray(refs)) fail("invalid mask transition references");
      for (const ref of refs) {
        if (!maskIds.has(ref) || changingMasks.has(ref))
          fail(`invalid or multiply controlled mask ${ref}`);
        changingMasks.add(ref);
      }
    }
    const links = transition.doorLinks;
    if (links !== undefined) {
      if (
        !links ||
        !["trigger-transition", "swap-rights"].includes(links.mode) ||
        !Array.isArray(links.ids) ||
        !links.ids.length ||
        new Set(links.ids).size !== links.ids.length
      )
        fail("invalid transition door links");
      const doors = [...data.doors, ...(data.interiors ?? []).flatMap((room) => room.doors)];
      for (const id of links.ids) {
        if (typeof id !== "string" || !doors.some((door) => door.id === id))
          fail(`transition references missing ordinary/interior door ${id}`);
        if (links.mode === "trigger-transition") {
          if (triggeringDoors.has(id)) fail(`door ${id} triggers multiple transitions`);
          triggeringDoors.add(id);
        }
      }
    }
    for (const refs of [transition.initialSight, transition.appliedSight]) {
      if (refs === undefined) continue;
      if (!Array.isArray(refs)) fail("invalid sight transition references");
      for (const ref of refs) {
        if (
          typeof ref !== "string" ||
          changingSight.has(ref) ||
          disabledParts.has(ref) ||
          !(
            data.volumes?.some((v) => v.id === ref) ||
            (data.collision === "parts" && nodes.has(ref))
          )
        )
          fail(`invalid or multiply controlled sight obstacle ${ref}`);
        changingSight.add(ref);
      }
    }
    for (const contour of [transition.applyPolygon, transition.noApplyPolygon])
      if (!(Array.isArray(contour) && contour.length === 0)) polygon(contour);
  }
  if (data.movementSolids !== undefined) {
    if (
      !Array.isArray(data.movementSolids) ||
      new Set(data.movementSolids).size !== data.movementSolids.length
    )
      fail("invalid permanent movement solids");
    for (const ref of data.movementSolids)
      if (
        typeof ref !== "string" ||
        disabledParts.has(ref) ||
        !(
          data.volumes?.some((volume) => volume.id === ref && volume.shape.solid) ||
          (data.collision === "parts" &&
            descriptor.parts.some((part) => part.node === ref && part.obstacle_local_game?.solid))
        )
      )
        fail(`invalid permanent movement solid ${ref}`);
  }
  if (
    changingSight.size &&
    data.movementBlockers === undefined &&
    data.movementSolids === undefined
  )
    fail(
      "Sight transitions require explicit movement blockers or permanent movement solids; author navigation changes independently",
    );
  const integer = (n: unknown, max: number): n is number =>
    typeof n === "number" && Number.isInteger(n) && n >= 0 && n <= max;
  if (data.jumpZones !== undefined && !Array.isArray(data.jumpZones)) fail("invalid jump zones");
  if (data.jumpPairs !== undefined && !Array.isArray(data.jumpPairs)) fail("invalid jump pairs");
  const jumpZones = new Set<string>();
  for (const zone of data.jumpZones ?? []) {
    feature(zone);
    if (
      !point(zone.anchor, 3) ||
      typeof zone.helperNeeded !== "boolean" ||
      !Array.isArray(zone.polygon) ||
      zone.polygon.length < 3 ||
      !zone.polygon.every((p) => point(p, 3))
    )
      fail(`invalid jump zone ${zone.id}`);
    jumpZones.add(zone.id);
  }
  const usedJumpZones = new Set<string>();
  if (data.jumpSegments !== undefined && !Array.isArray(data.jumpSegments))
    fail("invalid jump segments");
  for (const segment of data.jumpSegments ?? []) {
    feature(segment);
    const edge = segment.edge;
    if (
      typeof segment.long !== "boolean" ||
      (segment.join !== undefined && !point(segment.join, 3)) ||
      (segment.join === undefined && segment.attachment === undefined) ||
      !edge ||
      !jumpZones.has(edge.zone) ||
      !point(edge.a, 3) ||
      !point(edge.b, 3)
    )
      fail(`invalid jump segment or missing zone ${segment.id}`);
    usedJumpZones.add(edge.zone);
    if (segment.attachment !== undefined) {
      const rules = segment.attachment;
      if (
        !rules ||
        ![rules.maxGap, rules.maxRise, rules.maxDrop, rules.minOverlap].every(Number.isFinite) ||
        rules.maxGap <= 0 ||
        rules.maxRise < 0 ||
        rules.maxDrop < 0 ||
        rules.minOverlap <= 0
      )
        fail(`invalid jump attachment ${segment.id}`);
      if (
        rules.clearance &&
        (![rules.clearance.radius, rules.clearance.height].every(Number.isFinite) ||
          rules.clearance.radius < 0 ||
          rules.clearance.height < 0)
      )
        fail(`invalid jump clearance ${segment.id}`);
    }
  }
  for (const pair of data.jumpPairs ?? []) {
    feature(pair);
    if (typeof pair.long !== "boolean" || !Array.isArray(pair.edges) || pair.edges.length !== 2)
      fail(`invalid jump pair ${pair.id}`);
    for (const edge of pair.edges) {
      if (!edge || !jumpZones.has(edge.zone) || !point(edge.a, 3) || !point(edge.b, 3))
        fail(`invalid jump edge or missing zone ${pair.id}`);
      usedJumpZones.add(edge.zone);
    }
  }
  if ([...jumpZones].some((id) => !usedJumpZones.has(id))) fail("jump zone has no paired edge");
  if (data.lights !== undefined && !Array.isArray(data.lights)) fail("invalid light regions");
  for (const light of data.lights ?? []) {
    feature(light);
    if (
      light.receiverGroup !== undefined &&
      (typeof light.receiverGroup !== "string" || !light.receiverGroup.trim())
    )
      fail(`invalid light receiver group ${light.id}`);
    if (
      !integer(light.ambiences, 4294967295) ||
      !Array.isArray(light.polygon) ||
      light.polygon.length < 3 ||
      !light.polygon.every((p) => point(p, 3))
    )
      fail(`invalid light region ${light.id}`);
    if (
      light.receivers !== undefined &&
      (!Array.isArray(light.receivers) ||
        !light.receivers.length ||
        !light.receivers.every((p) => point(p, 3)))
    )
      fail(`invalid light receivers ${light.id}`);
    if (
      light.receiverSegments !== undefined &&
      (!Array.isArray(light.receiverSegments) ||
        !light.receiverSegments.length ||
        !light.receiverSegments.every(
          (segment) =>
            Array.isArray(segment) &&
            segment.length === 2 &&
            segment.every((p) => point(p, 3)) &&
            segment[0].some((v, i) => v !== segment[1][i]),
        ))
    )
      fail(`invalid light receiving segments ${light.id}`);
    if (
      light.receiverPolylines !== undefined &&
      (!Array.isArray(light.receiverPolylines) ||
        !light.receiverPolylines.length ||
        !light.receiverPolylines.every(
          (line) =>
            Array.isArray(line) &&
            line.length >= 2 &&
            line.every((p) => point(p, 3)) &&
            line.slice(1).every((p, i) => p.some((v, j) => v !== line[i]![j])),
        ))
    )
      fail(`invalid light receiving polylines ${light.id}`);
  }
  if (data.animations !== undefined && !Array.isArray(data.animations))
    fail("invalid scenery animations");
  for (const animation of data.animations ?? []) {
    feature(animation);
    if (
      animation.resourceDirectory !== undefined &&
      (!safeLibraryPath(animation.resourceDirectory) ||
        !animation.resourceDirectory.endsWith(".rhs.d") ||
        !/^[a-zA-Z0-9_-]+(?:\.rhs)?$/i.test(animation.file))
    )
      fail(`invalid scenery resource directory ${animation.id}`);
    if (
      !point(animation.anchor, 3) ||
      !point(animation.center, 2) ||
      typeof animation.file !== "string" ||
      !animation.file.trim() ||
      !animation.file.replace(/\.rhs$/i, "").trim() ||
      typeof animation.profile !== "string" ||
      !animation.profile.trim() ||
      typeof animation.active !== "boolean" ||
      typeof animation.forceDisplay !== "boolean" ||
      typeof animation.shadow !== "boolean" ||
      !Array.isArray(animation.displayPolyline) ||
      !animation.displayPolyline.every((p) => point(p, 3))
    )
      fail(`invalid scenery animation ${animation.id}`);
  }
  if (data.sounds !== undefined && !Array.isArray(data.sounds)) fail("invalid sound sources");
  for (const sound of data.sounds ?? []) {
    feature(sound);
    if (
      !integer(sound.sample, 2147483647) ||
      !integer(sound.kind, 3) ||
      !integer(sound.altitude, 3) ||
      !integer(sound.ambiences, 4294967295) ||
      typeof sound.active !== "boolean"
    )
      fail(`invalid sound source ${sound.id}`);
    if (
      sound.kind === 2
        ? !Array.isArray(sound.delay) ||
          sound.delay.length !== 3 ||
          !sound.delay.every((n) => integer(n, 65535)) ||
          sound.delay[0] > sound.delay[1] ||
          sound.delay[2] === 65535
        : sound.delay !== undefined
    )
      fail(`invalid sound delay ${sound.id}`);
    const s = sound.spatial;
    if (
      s !== undefined &&
      (!s ||
        !Array.isArray(s.polyline) ||
        !s.polyline.length ||
        !s.polyline.every((p) => point(p, 3)) ||
        !integer(s.innerDistance, 65535) ||
        !integer(s.outerDistance, 65535) ||
        s.innerDistance > s.outerDistance ||
        !integer(s.innerVolume, 100) ||
        !integer(s.outerVolume, 100) ||
        !integer(s.noiseCoveringDistance, 65535))
    )
      fail(`invalid sound geometry ${sound.id}`);
  }
  if (data.volumes !== undefined && !Array.isArray(data.volumes)) fail("invalid gameplay volumes");
  const volumes = new Set<string>();
  for (const volume of data.volumes ?? []) {
    feature(volume);
    if (
      volume.movementHeadroom !== undefined &&
      (!Number.isFinite(volume.movementHeadroom) || volume.movementHeadroom < 0)
    )
      fail(`invalid movement headroom ${volume.id}`);
    if (nodes.has(volume.id)) fail("gameplay volume IDs must not shadow part nodes");
    volumes.add(volume.id);
    const shape = volume.shape;
    if (
      !shape ||
      !Array.isArray(shape.points) ||
      shape.points.length < 3 ||
      !shape.points.every(
        (p) => p && [p.x, p.y, p.z_bottom, p.z_top].every(Number.isFinite) && p.z_bottom <= p.z_top,
      ) ||
      ![shape.solid, shape.opaque, shape.mouse, shape.show_shadow_polygon].every(
        (v) => typeof v === "boolean",
      ) ||
      !Number.isInteger(shape.default_material) ||
      shape.default_material < 0 ||
      shape.default_material > 9 ||
      "projection_area" in shape ||
      "material_indices" in shape
    )
      fail(`invalid gameplay volume ${volume.id}`);
  }
  if (data.projectionReceivers !== undefined && !Array.isArray(data.projectionReceivers))
    fail("invalid projection receivers");
  const receiverVolumes = new Set<string>();
  for (const receiver of data.projectionReceivers ?? []) {
    feature(receiver);
    if (
      receiver.navigationHeight !== undefined &&
      (!Number.isFinite(receiver.navigationHeight) || receiver.receiverSegment !== undefined)
    )
      fail(`invalid projection navigation height ${receiver.id}`);
    if (
      receiver.receiverSegment !== undefined &&
      (!Array.isArray(receiver.receiverSegment) ||
        receiver.receiverSegment.length !== 2 ||
        !receiver.receiverSegment.every((p) => point(p, 3)) ||
        !receiver.receiverSegment[0].some((v, i) => v !== receiver.receiverSegment![1][i]))
    )
      fail(`invalid projection receiving segment ${receiver.id}`);
    if (
      !point(receiver.anchor, 3) ||
      disabledParts.has(receiver.volume) ||
      receiverVolumes.has(receiver.volume) ||
      data.surfaces.some((surface) => surface.projectionVolume === receiver.volume) ||
      !(
        volumes.has(receiver.volume) ||
        (data.collision === "parts" &&
          descriptor.parts.some(
            (part) =>
              part.node === receiver.volume &&
              part.obstacle_local_game &&
              part.mission_profile === undefined,
          ))
      )
    )
      fail(`invalid projection receiver ${receiver.id}`);
    receiverVolumes.add(receiver.volume);
  }
  if (data.materials !== undefined && !Array.isArray(data.materials)) fail("invalid materials");
  for (const region of data.materials ?? []) {
    feature(region);
    if (
      !Array.isArray(region.polygon) ||
      region.polygon.length < 3 ||
      !region.polygon.every((p) => point(p, 3)) ||
      !Number.isInteger(region.material) ||
      region.material < 0 ||
      region.material > 9 ||
      typeof region.ground !== "boolean" ||
      !Array.isArray(region.obstacles) ||
      region.obstacles.some((node) => !nodes.has(node) && !volumes.has(node)) ||
      new Set(region.obstacles).size !== region.obstacles.length ||
      (!region.ground &&
        !region.obstacles.length &&
        !data.surfaces.some(
          (surface) =>
            Array.isArray(surface.projectionMaterials?.regions) &&
            surface.projectionMaterials.regions.includes(region.id),
        ))
    )
      fail(`invalid material region ${region.id}`);
    if (
      region.obstacles.some(
        (node) => disabledParts.has(node) || (nodes.has(node) && data.collision !== "parts"),
      )
    )
      fail(`material region ${region.id} references disabled obstacles`);
  }
  if (data.movementBlockers !== undefined && !Array.isArray(data.movementBlockers))
    fail("invalid movement blockers");
  if (data.movementClearances !== undefined && !Array.isArray(data.movementClearances))
    fail("invalid movement clearances");
  for (const surface of [
    ...data.surfaces,
    ...(data.movementBlockers ?? []),
    ...(data.movementClearances ?? []),
    ...(data.movementTransitions ?? []).flatMap((t) => [...t.initial, ...t.applied]),
  ]) {
    feature(surface);
    polygon(surface.polygon);
    if (
      surface.navigationHeight !== undefined &&
      (!Number.isFinite(surface.navigationHeight) ||
        (!data.movementClearances?.includes(surface) && !data.movementBlockers?.includes(surface)))
    )
      fail(`invalid collision navigation height on ${surface.id}`);
    if (surface.terrainReach !== undefined) {
      const reach = surface.terrainReach;
      if (
        !reach ||
        ![reach.below, reach.above].every(Number.isFinite) ||
        reach.below < 0 ||
        reach.above < 0 ||
        reach.below + reach.above <= 0 ||
        !(data.movementTransitions ?? []).some(
          (t) => t.initial.includes(surface) || t.applied.includes(surface),
        )
      )
        fail(`invalid transition terrain reach on ${surface.id}`);
    }
    if (surface.jump !== undefined) {
      const jump = surface.jump;
      if (
        !jump ||
        !data.surfaces.includes(surface) ||
        data.lifts?.some((lift) => lift.surface === surface.id) ||
        ![
          jump.inset,
          jump.landingDepth,
          jump.maxGap,
          jump.maxRise,
          jump.maxDrop,
          jump.minOverlap,
        ].every(Number.isFinite) ||
        jump.inset < 0 ||
        jump.landingDepth <= 0 ||
        jump.maxGap <= 0 ||
        jump.maxRise < 0 ||
        jump.maxDrop < 0 ||
        jump.minOverlap <= 0 ||
        (jump.maxLevelAdjustment !== undefined &&
          (!Number.isFinite(jump.maxLevelAdjustment) || jump.maxLevelAdjustment < 0)) ||
        (jump.edges !== undefined &&
          (!Array.isArray(jump.edges) ||
            new Set(jump.edges).size !== jump.edges.length ||
            jump.edges.some(
              (index) => !Number.isInteger(index) || index < 0 || index >= surface.polygon.length,
            ))) ||
        (jump.clearance !== undefined &&
          (!jump.clearance ||
            ![jump.clearance.radius, jump.clearance.height].every(Number.isFinite) ||
            jump.clearance.radius < 0 ||
            jump.clearance.height < 0))
      )
        fail(`invalid surface jump rules on ${surface.id}`);
    }
    if (
      surface.projectionVolume !== undefined &&
      (disabledParts.has(surface.projectionVolume) ||
        !data.surfaces.includes(surface) ||
        !(
          volumes.has(surface.projectionVolume) ||
          (data.collision === "parts" &&
            descriptor.parts.some(
              (part) =>
                part.node === surface.projectionVolume &&
                part.obstacle_local_game &&
                part.mission_profile === undefined,
            ))
        ) ||
        surface.projectionMaterials !== undefined)
    )
      fail(`invalid projection volume on ${surface.id}`);
    if (surface.projectionMaterials !== undefined) {
      const projection = surface.projectionMaterials;
      if (
        !data.surfaces.includes(surface) ||
        !projection ||
        !Number.isInteger(projection.defaultMaterial) ||
        projection.defaultMaterial < 0 ||
        projection.defaultMaterial > 9 ||
        (projection.priorityHeight !== undefined && !Number.isFinite(projection.priorityHeight)) ||
        (projection.priority !== undefined && !Number.isFinite(projection.priority)) ||
        (projection.planePoints !== undefined &&
          (!Array.isArray(projection.planePoints) ||
            projection.planePoints.length !== 3 ||
            !projection.planePoints.every((p) => point(p, 3)))) ||
        (projection.footprint !== undefined &&
          (!Array.isArray(projection.footprint) ||
            projection.footprint.length < 3 ||
            !projection.footprint.every((p) => point(p, 3)))) ||
        !Array.isArray(projection.regions) ||
        new Set(projection.regions).size !== projection.regions.length ||
        projection.regions.some((id) => !data.materials?.some((region) => region.id === id))
      )
        fail(`invalid projection materials on ${surface.id}`);
    }
    if (
      surface.navigationRegion !== undefined &&
      (typeof surface.navigationRegion !== "string" ||
        !surface.navigationRegion.trim() ||
        !data.surfaces.includes(surface) ||
        data.lifts?.some((lift) => lift.surface === surface.id))
    )
      fail("navigation regions require nonempty labels on ordinary walkable surfaces");
    if (
      surface.navigationJoins !== undefined &&
      (!surface.navigationRegion ||
        !Array.isArray(surface.navigationJoins) ||
        !surface.navigationJoins.length ||
        surface.navigationJoins.some(
          (edge) => !Array.isArray(edge) || edge.length !== 2 || !edge.every((p) => point(p, 3)),
        ))
    )
      fail("navigation joins require 3D edge sockets on labelled ordinary surfaces");
    if (
      surface.navigationJoinHeightTolerance !== undefined &&
      (!surface.navigationJoins ||
        !Number.isFinite(surface.navigationJoinHeightTolerance) ||
        surface.navigationJoinHeightTolerance < 0)
    )
      fail("navigation join height tolerance requires sockets and a nonnegative finite height");
    if (
      surface.navigationJoinMinimumOverlap !== undefined &&
      (!surface.navigationJoins ||
        !Number.isFinite(surface.navigationJoinMinimumOverlap) ||
        surface.navigationJoinMinimumOverlap <= 0)
    )
      fail("navigation join minimum overlap requires sockets and a positive finite span");
    if (
      !(typeof surface.height === "number" && Number.isFinite(surface.height)) &&
      !(
        Array.isArray(surface.height) &&
        surface.height.length === surface.polygon.length &&
        surface.height.every((z) => typeof z === "number" && Number.isFinite(z))
      )
    )
      fail("invalid surface height");
    if (
      surface.preserveMovementPrecision !== undefined &&
      typeof surface.preserveMovementPrecision !== "boolean"
    )
      fail("invalid movement precision setting");
    if (
      surface.preserveMovementBoundary !== undefined &&
      (typeof surface.preserveMovementBoundary !== "boolean" ||
        (surface.preserveMovementBoundary &&
          (!surface.navigationRegion ||
            !data.surfaces.includes(surface) ||
            data.lifts?.some((l) => l.surface === surface.id))))
    )
      fail("preserved movement boundaries require labelled ordinary walkable surfaces");
    if (surface.holes !== undefined) {
      if (!Array.isArray(surface.holes)) fail("invalid surface holes");
      for (const hole of surface.holes) polygon(hole);
    }
    if (
      surface.movementContour !== undefined &&
      (typeof surface.movementContour !== "string" ||
        !surface.movementContour.trim() ||
        (!data.movementBlockers?.includes(surface) &&
          !data.movementTransitions?.some(
            (t) => t.initial.includes(surface) || t.applied.includes(surface),
          )))
    )
      fail("movement contour labels require explicit movement blockers or transition contours");
    if (
      surface.holeContours !== undefined &&
      (!surface.preserveMovementBoundary ||
        !Array.isArray(surface.holeContours) ||
        surface.holeContours.length !== surface.holes?.length ||
        surface.holeContours.some((id) => typeof id !== "string" || !id.trim()))
    )
      fail("hole contour labels must match preserved boundary holes");
  }
  const validateDoor = (door: AssetDoor, kind: "ordinary" | "lift" | "interior") => {
    const lift = kind === "lift";
    feature(door);
    // A connection can have no clickable sector while still linking navigation areas.
    if (!(Array.isArray(door.polygon) && door.polygon.length === 0)) polygon(door.polygon);
    if (
      !point(door.outside, 3) ||
      !point(door.inside, 3) ||
      !point(door.middle, 3) ||
      !(lift ? [4, 5, 6] : kind === "interior" ? [1, 2] : [0, 3, 7]).includes(door.type) ||
      typeof door.locked !== "boolean" ||
      typeof door.unlockable !== "boolean"
    )
      fail(`invalid door ${door.id}`);
    for (const key of ["outsideAnchor", "insideAnchor"] as const)
      if (door[key] !== undefined && !point(door[key], 3)) fail(`invalid door ${door.id} ${key}`);
    if (kind === "interior" && door.insideAnchor !== undefined)
      fail(`interior door ${door.id} cannot override its shared room with an inside anchor`);
    if (
      door.outsideReceiverSegment !== undefined &&
      ((kind !== "interior" && !(kind === "ordinary" && door.type === 0)) ||
        door.outsideAnchor !== undefined ||
        !Array.isArray(door.outsideReceiverSegment) ||
        door.outsideReceiverSegment.length !== 2 ||
        !door.outsideReceiverSegment.every((p) => point(p, 3)) ||
        !door.outsideReceiverSegment[0].some((v, i) => v !== door.outsideReceiverSegment![1][i]))
    )
      fail(`invalid ${kind} door receiving segment ${door.id}`);
    if (
      door.insideReceiverSegment !== undefined &&
      (kind !== "ordinary" ||
        door.type !== 0 ||
        door.insideAnchor !== undefined ||
        !Array.isArray(door.insideReceiverSegment) ||
        door.insideReceiverSegment.length !== 2 ||
        !door.insideReceiverSegment.every((p) => point(p, 3)) ||
        !door.insideReceiverSegment[0].some((v, i) => v !== door.insideReceiverSegment![1][i]))
    )
      fail(`invalid ${kind} door inside receiving segment ${door.id}`);
    for (const key of ["active", "lockedVillains", "lockedCivilians", "allowContinuous"] as const)
      if (door[key] !== undefined && typeof door[key] !== "boolean")
        fail(`invalid door ${door.id} ${key}`);
    if (door.afterTransition !== undefined) {
      if (!door.afterTransition || typeof door.afterTransition !== "object")
        fail(`invalid door ${door.id} transition locks`);
      for (const key of ["locked", "unlockable", "lockedVillains", "lockedCivilians"] as const)
        if (typeof door.afterTransition[key] !== "boolean")
          fail(`invalid door ${door.id} transition ${key}`);
    }
    if (
      door.allowContinuous &&
      (kind !== "ordinary" ||
        door.type !== 0 ||
        door.polygon.length ||
        door.active === false ||
        door.locked ||
        door.unlockable ||
        door.lockedVillains ||
        door.lockedCivilians ||
        Object.values(door.afterTransition ?? {}).some(Boolean))
    )
      fail(`door ${door.id} cannot allow continuous navigation with interaction or restrictions`);
  };
  for (const door of data.doors) validateDoor(door, "ordinary");
  if (data.lifts !== undefined && !Array.isArray(data.lifts)) fail("invalid lifts");
  const liftSurfaces = new Set<string>();
  for (const lift of data.lifts ?? []) {
    feature(lift);
    const surface = data.surfaces.find((s) => s.id === lift.surface);
    if (!surface || surface.node !== lift.node || liftSurfaces.has(lift.surface))
      fail(`lift ${lift.id} needs its own surface on the same node`);
    liftSurfaces.add(lift.surface);
    if (
      ![1, 2, 3].includes(lift.type) ||
      !point(lift.direction, 2) ||
      Math.hypot(...lift.direction) < 1e-6
    )
      fail(`invalid lift type or direction: ${lift.id}`);
    if (
      lift.joins !== undefined &&
      (!Array.isArray(lift.joins) || !lift.joins.length || !lift.joins.every((p) => point(p, 3)))
    )
      fail(`invalid lift joins: ${lift.id}`);
    if (
      !Array.isArray(lift.doors) ||
      (!lift.joins && (lift.doors.length < 2 || !lift.doors.some((d) => d.type === 5)))
    )
      fail(`lift ${lift.id} needs at least two traversal doors including a low door`);
    for (const door of lift.doors) {
      if (door.node !== lift.node) fail(`lift ${lift.id} door must use its owning node`);
      validateDoor(door, "lift");
    }
  }
  if (data.interiors !== undefined && !Array.isArray(data.interiors)) fail("invalid interiors");
  for (const interior of data.interiors ?? []) {
    feature(interior);
    if (
      interior.joins !== undefined &&
      (!Array.isArray(interior.joins) ||
        !interior.joins.length ||
        interior.joins.some(
          (join) =>
            !join ||
            !point(join.point, 3) ||
            !point(join.direction, 2) ||
            Math.hypot(...join.direction) < 1e-6,
        ))
    )
      fail(`invalid interior joins: ${interior.id}`);
    if (!Array.isArray(interior.doors) || (!interior.doors.length && !interior.joins?.length))
      fail(`interior ${interior.id} has no entrance or passage socket`);
    for (const door of interior.doors) {
      if (door.node !== interior.node)
        fail(`interior ${interior.id} door must use its owning node`);
      validateDoor(door, "interior");
    }
  }
}
