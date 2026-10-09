import { terrainGameplay } from "./authored-terrain.ts";
import { compileNavigationGraph } from "./compile-navigation-graph.ts";
import { placeGameplaySurface } from "./place-gameplay-surface.ts";
import { compilePhysicalStairRegion } from "./compile-physical-stair-region.ts";
import {
  containsNavigationAnchor,
  navigationAnchorHeight,
  onClippedReceivingBoundary,
  pointInGameplayPolygon as inside,
  type NavigationAnchorArea,
} from "./navigation-anchor.ts";
import { wallSplineGameplay } from "./wall-spline-gameplay.ts";
import polygonClipping, { type Polygon, type MultiPolygon } from "polygon-clipping";
import { assembleSightVolumes } from "./assemble-sight-volumes.ts";
import { orderSightVolumes } from "./order-sight-volumes.ts";
import { compileSoundSource } from "./compile-sound-source.ts";
import { compileSceneryAnimation } from "./compile-scenery-animation.ts";
import { fixedPolygonBoolean } from "./fixed-polygon-boolean.ts";
import { subtractMovementCollision } from "./subtract-movement-collision.ts";
import { unionMovementSurfaces } from "./union-movement-surfaces.ts";
import {
  assembleNavigationRegions,
  DisconnectedLiftRegion,
  type NavigationPiece,
} from "./assemble-navigation-regions.ts";
import { allocateLightReceivingLayers } from "./allocate-light-receiving-layers.ts";
import { compactNavigationLayers } from "./compact-navigation-layers.ts";
import { lightReceiverIntersection } from "./light-receiver-segment.ts";
import { preserveMovementBoundary } from "./preserve-movement-boundary.ts";
import {
  assembleNavigationJoins,
  orientNavigationJoin,
  type PlacedNavigationJoin,
} from "./assemble-navigation-joins.ts";
import { assembleJumpSegments, type PlacedJumpSegment } from "./assemble-jump-segments.ts";
import {
  createJumpClearance,
  mergeIntervals,
  type JumpReceivingSurface,
} from "./jump-clearance.ts";
import { auditCompiledJump } from "./audit-compiled-jump.ts";
import { createJumpWalkingClearance, type JumpWalkArea } from "./jump-walking-clearance.ts";
import {
  generateJumpLedges,
  jumpLandingBand,
  type JumpLandingBand,
} from "./generate-jump-ledges.ts";
import {
  assembleLiftSegments,
  UnavailableLiftJoin,
  type PlacedLiftSegment,
} from "./assemble-lift-segments.ts";
import { assembleInteriors, type PlacedInterior } from "./assemble-interiors.ts";
import { interiorEndpointId, validateInteriorConnections } from "./interior-connections.ts";
import {
  partitionProjectionMaterials,
  type ProjectionMaterialSupport,
} from "./partition-projection-materials.ts";
import { nativeReceiverGeometry } from "./native-receiver-geometry.ts";
import { partMatrix, transformedObstacle, type Level3D, type Level3DObject } from "./level3d.ts";
import { gameToScene, type Vec3 } from "./scene.ts";
import { sceneToGame } from "./geometry.ts";
import type { Point, SightObstacle } from "./level.ts";
import type { ProjectionAssetDescriptor } from "./projection-assets.ts";
import {
  validateAssetGameplay,
  type GameplayAssetDescriptor,
  type CompiledAssetGeometry,
  type AssetGameplay,
} from "./asset-gameplay.ts";

import {
  heightPlane,
  planeHeight,
  projectionPlaneAnchors,
  type HeightPlane,
} from "./gameplay-plane.ts";
import {
  movementVolumeHeightSlice,
  MOVEMENT_CONTACT_TOLERANCE,
} from "./movement-volume-height-slice.ts";
import { quantizeGeneratedMotionPolygon, simplifyMotionRing } from "./motion-quantization.ts";
import { restoreReceivingBoundary, restoreObstacleBoundary } from "./restore-receiving-boundary.ts";
import {
  motionBoundsKey,
  indexPreciseBlockers,
  joinedBlockedCoverage as recoverJoinedCoverage,
  joinedReceivingBoundary,
} from "./precise-movement-contours.ts";
import { normalizeGeneratedMotion } from "./normalize-generated-motion.ts";
import { normalizeGameplayStateViews } from "./gameplay-state-views.ts";
import { compileAppearanceBindings } from "./compile-appearance-bindings.ts";
import { assembleTransitions, type PlacedTransitionJoin } from "./assemble-transitions.ts";
import {
  maskBoundaryPolyline,
  rasterizeMaskGeometry,
  type MaskTriangle,
} from "./compile-mask-geometry.ts";
import {
  compileTransitionObstacles,
  MovementTransitionLimit,
  type PlacedTransitionBlocker,
} from "./compile-movement-transitions.ts";

type Instance = {
  id: string;
  descriptor: GameplayAssetDescriptor;
  parts: Map<string, Level3DObject>;
  frames: Map<string, Level3DObject>;
  background: boolean;
};
const signedArea = (ring: Point[]) =>
  ring.reduce((sum, a, i) => {
    const b = ring[(i + 1) % ring.length]!;
    return sum + a[0] * b[1] - b[0] * a[1];
  }, 0) / 2;
const narrowMovementRing = (ring: Point[]) =>
  Math.abs(signedArea(ring)) <=
  ring.reduce((sum, p, i) => {
    const q = ring[(i + 1) % ring.length]!;
    return sum + Math.hypot(q[0] - p[0], q[1] - p[1]);
  }, 0) /
    2;
function ring(points: Point[], label = "Gameplay polygon", minimumArea = 0.5): Point[] {
  // Plane construction in the runtime uses the first three vertices.
  // Remove straight-edge vertices introduced by polygon unions and clipping.
  const result = simplifyMotionRing(points);
  if (result.length < 3 || Math.abs(signedArea(result)) < minimumArea)
    throw new Error(
      `${label} collapses after coordinate quantization (${JSON.stringify(points.slice(0, 8))})`,
    );
  // Consistent winding is required by movement edge authorization.
  if (signedArea(result) < 0) result.reverse();
  return result;
}
const polygon = (points: Point[]): Polygon => [[...points, points[0]!]];
function instances(
  document: Level3D,
  descriptors: ReadonlyMap<string, ProjectionAssetDescriptor>,
): Instance[] {
  const result = new Map<string, Instance>();
  const hidden = new Set(document.groups.filter((g) => g.hidden).map((g) => g.id));
  for (const part of document.objects) {
    if (part.hidden || (part.group && hidden.has(part.group))) continue;
    const match = /^asset:([^:]+):(.+)$/.exec(part.node);
    if (!match)
      throw new Error(
        `Object ${part.id} has no asset definition. Publish its gameplay in an asset before compiling.`,
      );
    const descriptor = descriptors.get(match[1]!);
    if (!descriptor) throw new Error(`Missing pinned asset ${match[1]}`);
    const id = `${part.group ?? part.id}/${match[1]}`;
    let instance = result.get(id);
    if (!instance) {
      instance = { id, descriptor, parts: new Map(), frames: new Map(), background: false };
      result.set(id, instance);
    }
    if (instance.parts.has(match[2]!))
      throw new Error(`Duplicate asset node ${part.node} in placement ${id}`);
    instance.parts.set(match[2]!, part);
  }
  // Explicit gameplay keeps its local frames even when a mesh part is hidden.
  // Entirely hidden placements still contribute no instance.
  for (const part of document.objects) {
    const match = /^asset:([^:]+):(.+)$/.exec(part.node);
    if (!match) continue;
    const instance = result.get(`${part.group ?? part.id}/${match[1]}`);
    if (!instance) continue;
    if (instance.frames.has(match[2]!))
      throw new Error(`Duplicate asset frame ${part.node} in placement ${instance.id}`);
    instance.frames.set(match[2]!, part);
  }
  for (const source of document.sceneAssets) {
    if (source.role !== "ground") continue;
    const descriptor = descriptors.get(source.id);
    if (!descriptor) throw new Error(`Ground asset ${source.id} has no pinned gameplay definition`);
    result.set(`ground/${source.id}`, {
      id: `ground/${source.id}`,
      descriptor,
      parts: new Map(),
      frames: new Map(),
      background: true,
    });
  }
  return [...result.values()].sort((a, b) => a.id.localeCompare(b.id, "en"));
}

export interface CompiledScenerySource {
  assetId: string;
  animationId: string;
}

/** Compile only placed editor assets. There is deliberately no datadir, source-map or level-record input. */
export function compileAssetGameplay(
  document: Level3D,
  descriptors: ReadonlyMap<string, ProjectionAssetDescriptor>,
  bounds: [number, number, number, number],
  options: {
    bestEffort?: boolean;
    onProgress?: (stage: string) => void;
    onSceneryCompiled?: (sources: CompiledScenerySource[]) => void;
  } = {},
): CompiledAssetGeometry {
  const omitted = new Set<string>();
  const fixedTransitions = new Set<string>();
  const omissions: string[] = [];
  // Terrain is independent of omitted asset controls and traversal assemblies.
  // Share it across retries of this export, but never across editor documents.
  let preparedTerrain: { value: ReturnType<typeof terrainGameplay> } | undefined;
  const getTerrain = (scene: Level3D) =>
    (preparedTerrain ??= { value: terrainGameplay(scene) }).value;
  for (;;) {
    try {
      const scenerySources: CompiledScenerySource[] = [];
      const result = compileAssetGameplayAttempt(
        document,
        descriptors,
        bounds,
        options,
        omitted,
        fixedTransitions,
        scenerySources,
        getTerrain,
      );
      if (omissions.length) result.warnings = [...omissions, ...(result.warnings ?? [])];
      options.onSceneryCompiled?.(scenerySources);
      return result;
    } catch (error) {
      if (
        error instanceof MovementTransitionLimit &&
        options.bestEffort &&
        !fixedTransitions.has(error.transition)
      ) {
        fixedTransitions.add(error.transition);
        omissions.push(
          `Transition ${error.transition}: control omitted; retained its initial state; ${error.message}`,
        );
        continue;
      }
      if (
        error instanceof UnavailableStateControl &&
        (options.bestEffort || error.cropped) &&
        !fixedTransitions.has(error.id)
      ) {
        fixedTransitions.add(error.id);
        omissions.push(
          `Transition ${error.id}: control omitted; retained its initial movement barriers, masks and door permissions; ${error.message}`,
        );
        continue;
      }
      if (
        !(error instanceof UnavailableLiftPlacement) ||
        (!options.bestEffort && !error.cropped) ||
        error.lifts.every((id) => omitted.has(id))
      )
        throw error;
      for (const id of error.lifts) omitted.add(id);
      omissions.push(
        `Lift ${error.placement}: traversal omitted because its assembly cannot connect after placement; retained asset collision and independent gameplay; ${error.message}`,
      );
    }
  }
}

class UnavailableLiftPlacement extends Error {
  readonly lifts: string[];
  readonly placement: string;
  readonly cropped: boolean;
  constructor(placement: string, message: string, cropped: boolean, lifts: string[]) {
    super(message);
    this.placement = placement;
    this.lifts = lifts;
    this.cropped = cropped;
  }
}

class UnavailableStateControl extends Error {
  readonly id: string;
  readonly cropped: boolean;
  constructor(id: string, message: string, cropped: boolean) {
    super(message);
    this.id = id;
    this.cropped = cropped;
  }
}

function compileAssetGameplayAttempt(
  document: Level3D,
  descriptors: ReadonlyMap<string, ProjectionAssetDescriptor>,
  bounds: [number, number, number, number],
  options: { bestEffort?: boolean; onProgress?: (stage: string) => void },
  omitted: ReadonlySet<string>,
  fixedTransitions: ReadonlySet<string>,
  scenerySources: CompiledScenerySource[],
  getTerrain: (document: Level3D) => ReturnType<typeof terrainGameplay>,
): CompiledAssetGeometry {
  const warnings: string[] = [];
  options.onProgress?.("Preparing asset placements");
  ({ document, descriptors } = normalizeGameplayStateViews(document, descriptors));
  // Saved placements may contain old obstacle snapshots. Geometry authority is
  // the pinned asset; scene instances supply identity, visibility and transforms.
  document = {
    ...document,
    objects: document.objects.map((part) => {
      const match = /^asset:([^:]+):(.+)$/.exec(part.node);
      if (!match) return part;
      const definition = descriptors.get(match[1]!)?.parts.find((p) => p.node === match[2]);
      if (!definition) throw new Error(`Missing asset part definition ${part.node}`);
      return { ...part, obstacle: definition.obstacle_local_game };
    }),
  };
  let placements = instances(document, descriptors);
  options.onProgress?.("Constructing terrain");
  const terrain = getTerrain(document);
  options.onProgress?.("Constructing spline walls");
  const walls = wallSplineGameplay(document, descriptors, !!options.bestEffort, bounds);
  warnings.push(...walls.warnings);
  for (const descriptor of walls.descriptors)
    placements.push({
      id: descriptor.id,
      descriptor,
      parts: new Map(),
      frames: new Map(),
      background: true,
    });
  if (terrain)
    placements.push({
      id: "authored-terrain",
      descriptor: terrain,
      parts: new Map(),
      frames: new Map(),
      background: true,
    });
  const missing = [
    ...new Set(placements.filter((p) => !p.descriptor.gameplay).map((p) => p.descriptor.id)),
  ];
  if (missing.length && !options.bestEffort)
    throw new Error(
      `Missing asset gameplay definitions (${missing.length}): ${missing.join(", ")}. Add local surfaces and door definitions to these assets; no source-level fallback is available.`,
    );
  if (options.bestEffort) {
    for (const id of missing)
      warnings.push(`Asset ${id}: gameplay omitted because no local gameplay definition exists.`);
    placements = placements.filter((placement) => placement.descriptor.gameplay);
    if (document.population?.actors.length || document.population?.items.length)
      warnings.push(
        "Mission population omitted from map gameplay; retained in the embedded editor document.",
      );
  }
  if (
    !options.bestEffort &&
    (document.population?.actors.length || document.population?.items.length)
  )
    throw new Error(
      "Map compilation does not support embedded mission population; keep NPCs and items in a separate mission",
    );
  if (document.groups.some((g) => g.states) && !options.bestEffort)
    throw new Error(
      "Asset state transitions need gameplay compilation support before this map can be exported",
    );
  if (options.bestEffort)
    for (const group of document.groups)
      if (group.states)
        warnings.push(
          `Group ${group.id}: manually authored state controls are unsupported; exported in its initial visual state.`,
        );
  const transitionJoins = new Map<string, PlacedTransitionJoin>();
  const project = (p: Vec3): Point => [quantize(p[0]), quantize(p[1] - p[2])];
  const warnedDraftAssets = new Set<string>();
  const surfaces: {
    owner: string;
    polygon: Point[];
    holes: Point[][];
    plane: HeightPlane;
    worldPlane: HeightPlane;
    worldPolygon: Point[];
    worldHoles: Point[][];
    lift?: string;
    navigationRegion?: string;
    acceptsNavigationJoins?: boolean;
    preserveMovementBoundary?: boolean;
    holeContours?: string[];
    movementContour?: string;
  }[] = [];
  const movementBlockers: typeof surfaces = [];
  const projectionReceivers: {
    id: string;
    anchor: Vec3;
    receiverSegment?: [Vec3, Vec3];
    shape: SightObstacle;
  }[] = [];
  const navigationJoins: PlacedNavigationJoin[] = [];
  const movementSolids: { owner: string; shape: SightObstacle; headroom?: number }[] = [];
  const movementClearances: typeof surfaces = [];
  const transitionBlockers: (PlacedTransitionBlocker & {
    worldPlane: HeightPlane;
    worldPolygon: Point[];
    worldHoles: Point[][];
  })[] = [];
  const projectionSupports: (ProjectionMaterialSupport & {
    plane: HeightPlane;
    holes: Point[][];
    navigationRegion?: string;
    lift?: string;
  })[] = [];
  const lights: {
    id: string;
    receiverGroup?: string;
    polygon: Point[];
    receiverContour: Point[];
    plane: HeightPlane;
    ambiences: number;
    receivers?: Vec3[];
    receiverSegments?: [Vec3, Vec3][];
    receiverPolylines?: Vec3[][];
  }[] = [];
  const placedMasks: {
    id: string;
    anchor: Vec3;
    receiverPoints?: Vec3[];
    receiverSegment?: [Vec3, Vec3];
    receiverPolyline?: Vec3[];
    receiverPolylines?: Vec3[][];
    triangles: MaskTriangle[];
    alphaCoverage?: import("./mask-alpha-sampler.ts").MaskAlphaCoverage;
    cullBackfaces?: boolean;
    rules: Omit<import("./level.ts").Mask, "layer" | "box_top_left" | "box_size" | "mask_data">;
  }[] = [];
  const jumpZones: { id: string; polygon: Point[]; anchor: Vec3; helper: boolean }[] = [];
  const jumpSegments: PlacedJumpSegment[] = [];
  const generatedLandings = new Map<string, JumpLandingBand>();
  const generatedJumpZoneIds = new Set<string>();
  const jumpPairs: { id: string; long: boolean; edges: { zone: string; a: Vec3; b: Vec3 }[] }[] =
    [];
  const transitions: {
    id: string;
    hasAppearance: boolean;
    waypoint: Vec3;
    waypointAnchor: Vec3;
    waypointReceiverSegment?: [Vec3, Vec3];
    active: boolean;
    definitive: boolean;
    applyPolygon: Point[];
    noApplyPolygon: Point[];
    changes: { layer: number; sector: number; changing_obstacle: number }[];
    initialSight: number[];
    appliedSight: number[];
    initialMasks: string[];
    appliedMasks: string[];
    doorLinks?: { mode: "trigger-transition" | "swap-rights"; ids: string[] };
  }[] = [];
  let lifts: PlacedLiftSegment[] = [];
  const placedInteriors: PlacedInterior[] = [];
  const doors: {
    name: string;
    lift?: string;
    interior?: string;
    definition: AssetGameplay["doors"][number];
    outside: Vec3;
    inside: Vec3;
    outsideAnchor: Vec3;
    insideAnchor: Vec3;
    outsideReceiverSegment?: [Vec3, Vec3];
    insideReceiverSegment?: [Vec3, Vec3];
    middle: Point;
    worldMiddle: Vec3;
    polygon: Point[];
  }[] = [];
  const sight: SightObstacle[] = [];
  const sightJoins: import("./assemble-sight-volumes.ts").PlacedSightJoin[] = [];
  const sightOrders = new Map<number, number>();
  const sightCaps: import("./assemble-sight-volumes.ts").PlacedSightCap[] = [];
  const materials: NonNullable<CompiledAssetGeometry["material_sectors"]> = [];
  const groundMaterials: number[] = [];
  const sounds: NonNullable<CompiledAssetGeometry["sound_sources"]> = [];
  const animations: NonNullable<CompiledAssetGeometry["animations"]> = [];
  let mapSettings: CompiledAssetGeometry["map_settings"];
  const quantize = (n: number) => {
    const result = Math.round(n);
    if (!Number.isFinite(n) || result < -32768 || result > 32767)
      throw new Error("Asset gameplay exceeds signed 16-bit coordinates");
    return result;
  };
  options.onProgress?.("Transforming asset gameplay");
  for (const placement of placements) {
    let authored = placement.descriptor.gameplay!;
    // Merged asset drafts can repeat advisory text. Best-effort export keeps
    // every distinct warning without treating repetition as broken geometry.
    if (options.bestEffort && authored.draft && Array.isArray(authored.draft.issues)) {
      const issues = [...new Set(authored.draft.issues)];
      if (issues.length !== authored.draft.issues.length)
        authored = { ...authored, draft: { ...authored.draft, issues } };
    }
    validateAssetGameplay(authored, placement.descriptor);
    const unavailableSurfaces = new Set(
      (authored.lifts ?? [])
        .filter((lift) => omitted.has(`${placement.id}/${lift.id}`))
        .map((lift) => lift.surface),
    );
    const gameplay = unavailableSurfaces.size
      ? {
          ...authored,
          lifts: authored.lifts?.filter((lift) => !omitted.has(`${placement.id}/${lift.id}`)),
          surfaces: authored.surfaces.filter((surface) => !unavailableSurfaces.has(surface.id)),
        }
      : authored;
    const queryOrder = new Map(Object.entries(gameplay.sightOrder ?? {}));
    if (gameplay.draft && !warnedDraftAssets.has(placement.descriptor.id)) {
      warnedDraftAssets.add(placement.descriptor.id);
      for (const issue of gameplay.draft.issues)
        warnings.push(`Draft gameplay asset ${placement.descriptor.id}: ${issue}`);
    }
    if (gameplay.environment) {
      const settings = {
        forest_level: gameplay.environment.forest,
        default_material: gameplay.environment.defaultMaterial,
      };
      if (
        mapSettings &&
        (mapSettings.forest_level !== settings.forest_level ||
          mapSettings.default_material !== settings.default_material)
      )
        throw new Error("Terrain assets disagree on map environment defaults");
      mapSettings = settings;
    }
    const transform = (node: string, point: Vec3): Vec3 => {
      let p = point;
      if (!placement.background || node !== "$root") {
        const part = placement.frames.get(node);
        if (!part) throw new Error(`${placement.id}: gameplay node ${node} is hidden or missing`);
        const matrix = partMatrix(document.camera, document, part);
        const local = gameToScene(document.camera, ...point);
        const world = [0, 1, 2].map(
          (row) =>
            matrix[row]! * local[0] +
            matrix[4 + row]! * local[1] +
            matrix[8 + row]! * local[2] +
            matrix[12 + row]!,
        ) as Vec3;
        p = sceneToGame(document.camera, world);
      }
      return [p[0] - bounds[0], p[1] - bounds[1], p[2]];
    };
    for (const zone of gameplay.jumpZones ?? [])
      jumpZones.push({
        id: `${placement.id}/${zone.id}`,
        anchor: transform(zone.node, zone.anchor),
        polygon: ring(
          zone.polygon.map((p) => project(transform(zone.node, p))),
          `${placement.id}/${zone.id}`,
        ),
        helper: zone.helperNeeded,
      });
    for (const pair of gameplay.jumpPairs ?? [])
      jumpPairs.push({
        id: `${placement.id}/${pair.id}`,
        long: pair.long,
        edges: pair.edges.map((edge) => ({
          zone: `${placement.id}/${edge.zone}`,
          a: transform(pair.node, edge.a),
          b: transform(pair.node, edge.b),
        })),
      });
    for (const segment of gameplay.jumpSegments ?? [])
      jumpSegments.push({
        id: `${placement.id}/${segment.id}`,
        long: segment.long,
        ...(segment.join ? { join: transform(segment.node, segment.join) } : {}),
        ...(segment.attachment ? { attachment: segment.attachment } : {}),
        edge: {
          zone: `${placement.id}/${segment.edge.zone}`,
          a: transform(segment.node, segment.edge.a),
          b: transform(segment.node, segment.edge.b),
        },
      });
    for (const light of gameplay.lights ?? []) {
      const points = light.polygon.map((p) => transform(light.node, p));
      lights.push({
        id: `${placement.id}/${light.id}`,
        ...(light.receiverGroup ? { receiverGroup: `${placement.id}/${light.receiverGroup}` } : {}),
        polygon: ring(points.map(project), `${placement.id}/${light.id}`),
        receiverContour: ring(
          points.map(([x, y, z]): Point => [x, y - z]),
          `${placement.id}/${light.id} receiver coverage`,
          1e-8,
        ),
        plane: heightPlane(points.map(([x, y, z]): Vec3 => [x, y - z, z])),
        ambiences: light.ambiences,
        ...(light.receivers
          ? { receivers: light.receivers.map((p) => transform(light.node, p)) }
          : {}),
        ...(light.receiverSegments
          ? {
              receiverSegments: light.receiverSegments.map(([a, b]): [Vec3, Vec3] => [
                transform(light.node, a),
                transform(light.node, b),
              ]),
            }
          : {}),
        ...(light.receiverPolylines
          ? {
              receiverPolylines: light.receiverPolylines.map((line) =>
                line.map((p) => transform(light.node, p)),
              ),
            }
          : {}),
      });
    }
    for (const sound of gameplay.sounds ?? []) sounds.push(compileSoundSource(sound, transform));
    for (const animation of gameplay.animations ?? []) {
      try {
        animations.push(
          compileSceneryAnimation(animation, transform, (message) => {
            warnings.push(`Animation ${placement.id}/${animation.id}: ${message}.`);
          }),
        );
        scenerySources.push({ assetId: placement.descriptor.id, animationId: animation.id });
      } catch (error) {
        if (!options.bestEffort || !(error instanceof Error)) throw error;
        warnings.push(`Animation ${placement.id}/${animation.id} omitted: ${error.message}.`);
      }
    }
    const partSight = new Map<string, SightObstacle>();
    const movementSolid = (id: string) =>
      gameplay.movementSolids?.includes(id) ?? gameplay.movementBlockers === undefined;
    const explicitSight = new Set([
      ...(gameplay.projectionReceivers ?? []).map((receiver) => receiver.volume),
      ...gameplay.surfaces.flatMap((surface) =>
        surface.projectionVolume === undefined ? [] : [surface.projectionVolume],
      ),
      ...(gameplay.movementTransitions ?? []).flatMap((t) => [
        ...(t.initialSight ?? []),
        ...(t.appliedSight ?? []),
      ]),
      ...(gameplay.masks ?? []).flatMap((mask) => mask.obstacles),
    ]);
    if (gameplay.collision === "parts")
      for (const [node, part] of placement.frames) {
        if (!placement.parts.has(node) && !explicitSight.has(node)) continue;
        if (placement.descriptor.parts.find((p) => p.node === node)?.collision === "none") {
          if (explicitSight.has(node) || gameplay.movementSolids?.includes(node))
            throw new Error(
              `${placement.id}/${node}: disabled part collision cannot supply gameplay obstacle links`,
            );
          continue;
        }
        if (
          placement.descriptor.parts.find((p) => p.node === node)?.mission_profile !== undefined
        ) {
          if (explicitSight.has(node) || gameplay.movementSolids?.includes(node))
            throw new Error(
              `${placement.id}/${node}: preview bounds require an authored gameplay volume`,
            );
          continue;
        }
        if (!part.obstacle) continue;
        const shape = transformedObstacle(document, part);
        // Keep per-vertex heights and flags, but rebuild all map-wide references.
        sight.push({
          ...shape,
          points: shape.points.map((p) => ({ ...p, x: p.x - bounds[0], y: p.y - bounds[1] })),
          projection_area: null,
          material_indices: [],
        });
        partSight.set(node, sight.at(-1)!);
        const order = queryOrder.get(node);
        if (order !== undefined) sightOrders.set(sight.length - 1, order);
        for (const cap of placement.descriptor.parts.find((p) => p.node === node)
          ?.sight_join_caps ?? []) {
          if (explicitSight.has(node))
            throw new Error(`Linked sight volume cannot declare assembly caps: ${node}`);
          sightCaps.push({ index: sight.length - 1, cap });
        }
        for (const edge of placement.descriptor.parts.find((p) => p.node === node)
          ?.sight_join_edges ?? []) {
          if (explicitSight.has(node))
            throw new Error(`Linked sight volume cannot declare assembly seams: ${node}`);
          sightJoins.push({
            index: sight.length - 1,
            edge: [transform(node, edge[0]), transform(node, edge[1])],
          });
        }
        if (movementSolid(node)) movementSolids.push({ owner: placement.id, shape: sight.at(-1)! });
      }
    for (const volume of gameplay.volumes ?? []) {
      const shape: SightObstacle = {
        ...volume.shape,
        projection_area: null,
        material_indices: [],
        points: volume.shape.points.map((p) => {
          const bottom = transform(volume.node, [p.x, p.y, p.z_bottom]);
          const top = transform(volume.node, [p.x, p.y, p.z_top]);
          if (Math.hypot(bottom[0] - top[0], bottom[1] - top[1]) > 1e-5)
            throw new Error(`Gameplay volume ${volume.id} must remain vertical after placement`);
          return { x: top[0], y: top[1], z_bottom: bottom[2], z_top: top[2] };
        }),
      };
      sight.push(shape);
      partSight.set(volume.id, shape);
      const order = queryOrder.get(volume.id);
      if (order !== undefined) sightOrders.set(sight.length - 1, order);
      if (movementSolid(volume.id))
        movementSolids.push({ owner: placement.id, shape, headroom: volume.movementHeadroom });
    }
    for (const transition of gameplay.movementTransitions ?? []) {
      if (!fixedTransitions.has(`${placement.id}/${transition.id}`)) continue;
      for (const id of transition.initialSight ?? []) {
        const shape = partSight.get(id);
        if (!shape)
          throw new Error(`${placement.id}/${transition.id}: missing initial sight obstacle ${id}`);
        shape.initial_active = true;
      }
      for (const id of transition.appliedSight ?? []) {
        const shape = partSight.get(id);
        if (!shape)
          throw new Error(`${placement.id}/${transition.id}: missing applied sight obstacle ${id}`);
        shape.initial_active = false;
      }
    }
    for (const receiver of gameplay.projectionReceivers ?? []) {
      const shape = partSight.get(receiver.volume);
      if (!shape) throw new Error(`${receiver.id}: missing projection volume ${receiver.volume}`);
      heightPlane(shape.points.slice(0, 3).map((p): Vec3 => [p.x, p.y - p.z_top, p.z_top]));
      let anchor = transform(receiver.node, receiver.anchor);
      if (receiver.navigationHeight !== undefined) {
        const navigationLocal: Vec3[] = [
          [0, 0, receiver.navigationHeight],
          [100, 0, receiver.navigationHeight],
          [0, 100, receiver.navigationHeight],
        ];
        const navigationPlane = heightPlane(
          navigationLocal.map((point) => {
            const [wx, wy, wz] = transform(receiver.node, point);
            return [wx, wy - wz, wz];
          }),
        );
        const projected: Point = [anchor[0], anchor[1] - anchor[2]];
        const height = planeHeight(navigationPlane, projected);
        anchor = [projected[0], projected[1] + height, height];
      }
      projectionReceivers.push({
        id: `${placement.id}/${receiver.id}`,
        anchor,
        ...(receiver.receiverSegment
          ? {
              receiverSegment: [
                transform(receiver.node, receiver.receiverSegment[0]),
                transform(receiver.node, receiver.receiverSegment[1]),
              ] as [Vec3, Vec3],
            }
          : {}),
        shape,
      });
    }
    for (const id of gameplay.movementSolids ?? [])
      if (!partSight.has(id))
        throw new Error(`Permanent movement solid ${placement.id}/${id} is hidden or missing`);
    const unavailableAppliedMasks = new Set(
      (gameplay.movementTransitions ?? []).flatMap((t) =>
        fixedTransitions.has(`${placement.id}/${t.id}`) ? (t.appliedMasks ?? []) : [],
      ),
    );
    for (const mask of gameplay.masks ?? []) {
      if (unavailableAppliedMasks.has(mask.id)) continue;
      const boundary = (points: Vec3[] | undefined, projected: boolean, closed = true) => {
        if (!points) return null;
        const placed = points.map((point): Point => {
          const p = transform(mask.node, point);
          return projected ? project(p) : [quantize(p[0]), quantize(p[1])];
        });
        try {
          return maskBoundaryPolyline(placed, closed);
        } catch (error) {
          if (!options.bestEffort || !(error instanceof Error)) throw error;
          warnings.push(
            `Mask ${placement.id}/${mask.id}: ${projected ? "character" : "projectile"} boundary omitted: ${error.message}; this masking rule is incomplete.`,
          );
          return null;
        }
      };
      const characterBoundary = boundary(
        mask.characterBoundary,
        true,
        mask.characterBoundaryClosed,
      );
      const projectileBoundary = boundary(
        mask.projectileBoundary,
        false,
        mask.projectileBoundaryClosed,
      );
      placedMasks.push({
        id: `${placement.id}/${mask.id}`,
        anchor: transform(mask.node, mask.anchor),
        receiverPoints: mask.receiverPoints?.map((p) => transform(mask.node, p)),
        ...(mask.receiverPolylines
          ? {
              receiverPolylines: mask.receiverPolylines.map((line) =>
                line.map((p) => transform(mask.node, p)),
              ),
            }
          : {}),
        ...(mask.receiverPolyline
          ? { receiverPolyline: mask.receiverPolyline.map((p) => transform(mask.node, p)) }
          : {}),
        ...(mask.receiverSegment
          ? {
              receiverSegment: [
                transform(mask.node, mask.receiverSegment[0]),
                transform(mask.node, mask.receiverSegment[1]),
              ] as [Vec3, Vec3],
            }
          : {}),
        cullBackfaces: mask.cullBackfaces,
        alphaCoverage: mask.alphaCoverage,
        triangles: mask.triangles.map(([a, b, c]) => [
          transform(mask.node, a),
          transform(mask.node, b),
          transform(mask.node, c),
        ]),
        rules: {
          mask_type:
            (characterBoundary ? 1 : 0) |
            (projectileBoundary || mask.obstacles.length ? 2 : 0) |
            (mask.view ? 4 : 0) |
            (mask.obstacles.length ? 16 : 0),
          character_polyline: characterBoundary,
          projectile_polyline: projectileBoundary ?? (mask.obstacles.length ? [] : null),
          obstacle_indices: mask.obstacles.map((id) => {
            const shape = partSight.get(id);
            if (!shape)
              throw new Error(
                `${placement.id}/${mask.id}: mask obstacle ${id} is hidden or missing`,
              );
            const index = sight.indexOf(shape);
            if (index > 65535) throw new Error("Mask obstacle reference exceeds 16-bit indices");
            return index;
          }),
        },
      });
    }
    const materialIndices = new Map<string, number>();
    for (const region of gameplay.materials ?? []) {
      const index = materials.length;
      materialIndices.set(region.id, index);
      if (index > 65535) throw new Error("Too many asset material regions");
      materials.push({
        material: region.material,
        polygon: {
          points: ring(
            region.polygon.map((p) => project(transform(region.node, p))),
            `${placement.id}/${region.id}`,
          ),
        },
      });
      if (region.ground) groundMaterials.push(index);
      for (const node of region.obstacles) {
        const obstacle = partSight.get(node);
        if (!obstacle)
          throw new Error(
            `${placement.id}/${region.id}: material obstacle ${node} is hidden or missing`,
          );
        obstacle.material_indices.push(index);
      }
    }
    const dynamic = (gameplay.movementTransitions ?? []).flatMap((t) => [
      ...t.initial.map((surface) => ({
        surface,
        transition: `${placement.id}/${t.id}`,
        applied: false,
        fixed: fixedTransitions.has(`${placement.id}/${t.id}`),
      })),
      ...(fixedTransitions.has(`${placement.id}/${t.id}`) ? [] : t.applied).map((surface) => ({
        surface,
        transition: `${placement.id}/${t.id}`,
        applied: true,
        fixed: false,
      })),
    ]);
    for (const t of gameplay.movementTransitions ?? []) {
      if (fixedTransitions.has(`${placement.id}/${t.id}`)) continue;
      if (t.join)
        transitionJoins.set(`${placement.id}/${t.id}`, {
          key: t.join.key,
          point: transform(t.node, t.join.point),
        });
      const sightRefs = (refs: string[] = []) =>
        refs.map((id) => {
          const shape = partSight.get(id);
          if (!shape)
            throw new Error(`${placement.id}/${t.id}: sight obstacle ${id} is hidden or missing`);
          return sight.indexOf(shape);
        });
      const contour = (points: Point[]) =>
        points.length
          ? ring(
              points.map((p) => project(transform(t.node, [...p, t.waypoint[2]]))),
              `${placement.id}/${t.id}`,
            )
          : [];
      transitions.push({
        id: `${placement.id}/${t.id}`,
        hasAppearance: !!t.appearances?.length,
        waypoint: transform(t.node, t.waypoint),
        waypointAnchor: transform(t.node, t.waypointAnchor ?? t.waypoint),
        ...(t.waypointReceiverSegment
          ? {
              waypointReceiverSegment: [
                transform(t.node, t.waypointReceiverSegment[0]),
                transform(t.node, t.waypointReceiverSegment[1]),
              ] as [Vec3, Vec3],
            }
          : {}),
        active: t.active,
        definitive: t.definitive,
        applyPolygon: contour(t.applyPolygon),
        noApplyPolygon: contour(t.noApplyPolygon),
        changes: [],
        initialSight: sightRefs(t.initialSight),
        appliedSight: sightRefs(t.appliedSight),
        initialMasks: (t.initialMasks ?? []).map((id) => `${placement.id}/${id}`),
        appliedMasks: (t.appliedMasks ?? []).map((id) => `${placement.id}/${id}`),
        ...(t.doorLinks
          ? {
              doorLinks: {
                mode: t.doorLinks.mode,
                ids: t.doorLinks.ids.map((id) => `${placement.id}/${id}`),
              },
            }
          : {}),
      });
    }
    for (const surface of [
      ...gameplay.surfaces,
      ...(gameplay.movementBlockers ?? []),
      ...(gameplay.movementClearances ?? []),
      ...dynamic.map((d) => d.surface),
    ]) {
      const geometry = placeGameplaySurface(surface, transform);
      const { localPlane, points, navigationPoints, worldPlane } = geometry;
      const anchors = surface.projectionMaterials?.planePoints;
      if (anchors) {
        heightPlane(anchors);
        if (anchors.some(([x, y, z]) => Math.abs(planeHeight(localPlane, [x, y]) - z) > 1e-4))
          throw new Error(`${surface.id}: receiving plane anchors must lie on the surface`);
      }
      for (const edge of surface.navigationJoins ?? [])
        navigationJoins.push({
          region: `${placement.id}/${surface.navigationRegion}`,
          owner: placement.id,
          heightTolerance: surface.navigationJoinHeightTolerance,
          minimumOverlap: surface.navigationJoinMinimumOverlap,
          edge: orientNavigationJoin(points, [
            transform(surface.node, edge[0]),
            transform(surface.node, edge[1]),
          ]),
        });
      // Fit before integer quantization so height remains exact after placement.
      const plane = heightPlane(navigationPoints.map(([x, y, z]) => [x, y - z, z]));
      const target = gameplay.movementClearances?.includes(surface)
        ? movementClearances
        : gameplay.movementBlockers?.includes(surface)
          ? movementBlockers
          : surfaces;
      // Clearances are intermediate cutouts. Snapping their intersections before
      // clipping solids bends otherwise straight movement boundaries.
      const clearance = target === movementClearances;
      // Physical climbs need their authored landing seams before final grid
      // rounding; rounding individual surfaces can disconnect a rotated entry.
      const continuous =
        clearance ||
        surface.preserveMovementPrecision === true ||
        gameplay.lifts?.some((lift) => lift.type === 2 || lift.type === 3) === true;
      const projectMovement = continuous ? ([x, y, z]: Vec3): Point => [x, y - z] : project;
      const minimumArea = continuous ? 1e-8 : 0.5;
      const placed = {
        owner: placement.id,
        acceptsNavigationJoins:
          surface.acceptsNavigationJoins || placement.id === "authored-terrain",
        preserveMovementBoundary: surface.preserveMovementBoundary,
        holeContours: surface.holeContours,
        movementContour: surface.movementContour,
        navigationRegion:
          surface.navigationRegion === undefined
            ? undefined
            : `${placement.id}/${surface.navigationRegion}`,
        // Collision cutouts must use the same navigation plane for both their
        // contour and height; physical height would shift the projected hole.
        polygon: ring(
          (clearance ? navigationPoints : points).map(projectMovement),
          `${placement.id}/${surface.id}`,
          minimumArea,
        ),
        plane,
        worldPlane,
        worldPolygon: navigationPoints.map(([x, y]): Point => [x, y]),
        worldHoles: geometry.navigationHoles.map((hole) => hole.map(([x, y]): Point => [x, y])),
        ...(gameplay.lifts?.find((l) => l.surface === surface.id)
          ? { lift: `${placement.id}/${gameplay.lifts.find((l) => l.surface === surface.id)!.id}` }
          : {}),
        holes: (clearance ? geometry.navigationHoles : geometry.holes).map((hole) =>
          ring(hole.map(projectMovement), `${placement.id}/${surface.id} hole`, minimumArea),
        ),
      };
      const change = dynamic.find((d) => d.surface === surface);
      if (surface.jump) {
        const generated = generateJumpLedges(
          `${placement.id}/${surface.id}`,
          points.map(([x, y, z]): Point => [x, y - z]),
          placed.holes,
          plane,
          surface.jump,
        );
        jumpSegments.push(...generated.segments);
        for (const [id, band] of generated.landings) generatedLandings.set(id, band);
        warnings.push(...generated.warnings);
      }
      const receiver =
        surface.projectionVolume === undefined
          ? undefined
          : partSight.get(surface.projectionVolume);
      if (surface.projectionVolume !== undefined && !receiver)
        throw new Error(`${surface.id}: missing projection volume ${surface.projectionVolume}`);
      if (receiver) {
        const top = receiver.points.map((p): Vec3 => [p.x, p.y - p.z_top, p.z_top]);
        // Physical receivers retain every vertex, but their receiving plane is
        // defined only by the first three, independently of later vertex heights.
        const receiverPlane = heightPlane(top.slice(0, 3));
        if (points.some(([x, y, z]) => Math.abs(planeHeight(receiverPlane, [x, y - z]) - z) > 1e-4))
          throw new Error(`${surface.id}: projection volume top must lie on the surface`);
        const coverage = ring(
          top.map(([x, y]): Point => [x, y]),
          `${surface.id} projection volume`,
        );
        const walking = points.map(([x, y, z]): Point => [x, y - z]);
        if (!fixedPolygonBoolean("intersection", polygon(walking), [polygon(coverage)]).length)
          throw new Error(`${surface.id}: projection volume does not overlap its walking surface`);
      }
      if (gameplay.surfaces.includes(surface))
        projectionSupports.push({
          ...placed,
          ...(receiver ? { obstacleIndex: sight.indexOf(receiver) } : {}),
          ...(anchors
            ? {
                planePoints: [
                  transform(surface.node, anchors[0]),
                  transform(surface.node, anchors[1]),
                  transform(surface.node, anchors[2]),
                ] as [Vec3, Vec3, Vec3],
              }
            : {}),
          footprint: receiver
            ? ring(receiver.points.map((p): Point => [p.x, p.y - p.z_top]))
            : surface.projectionMaterials?.footprint
              ? ring(
                  surface.projectionMaterials.footprint.map((point): Point => {
                    const [x, y, z] = transform(surface.node, point);
                    return [x, y - z];
                  }),
                  `${placement.id}/${surface.id} receiving footprint`,
                )
              : undefined,
          defaultMaterial:
            receiver?.default_material ?? surface.projectionMaterials?.defaultMaterial ?? 0,
          materialIndices:
            receiver?.material_indices ??
            (surface.projectionMaterials?.regions ?? []).map((id) => materialIndices.get(id)!),
          materialSignature: JSON.stringify(
            receiver
              ? receiver.material_indices.map((id) => materials[id])
              : (surface.projectionMaterials?.regions ?? []).map(
                  (id) => materials[materialIndices.get(id)!],
                ),
          ),
          explicit: receiver !== undefined || surface.projectionMaterials !== undefined,
          tiePriority: surface.projectionMaterials?.priority ?? 0,
          priority: Math.fround(
            surface.projectionMaterials?.priorityHeight === undefined
              ? Math.max(...points.map((point) => point[2]))
              : transform(surface.node, [0, 0, surface.projectionMaterials.priorityHeight])[2],
          ),
        });
      if (change)
        transitionBlockers.push({
          ...placed,
          transition: change.transition,
          applied: change.applied,
          fixed: change.fixed,
          ...(surface.terrainReach
            ? {
                terrainVolume: {
                  ...surface.terrainReach,
                  plane: heightPlane(points),
                  polygon: points.map(([x, y]): Point => [x, y]),
                  holes: geometry.holes.map((hole) => hole.map(([x, y]): Point => [x, y])),
                },
              }
            : {}),
        });
      else target.push(placed);
    }
    const placeDoor = (door: AssetGameplay["doors"][number], lift?: string, interior?: string) =>
      doors.push({
        lift,
        interior,
        name: `${placement.id}/${door.id}`,
        definition: door,
        outside: transform(door.node, door.outside),
        inside: transform(door.node, door.inside),
        outsideAnchor: transform(door.node, door.outsideAnchor ?? door.outside),
        insideAnchor: transform(door.node, door.insideAnchor ?? door.inside),
        ...(door.outsideReceiverSegment
          ? {
              outsideReceiverSegment: [
                transform(door.node, door.outsideReceiverSegment[0]),
                transform(door.node, door.outsideReceiverSegment[1]),
              ] as [Vec3, Vec3],
            }
          : {}),
        ...(door.insideReceiverSegment
          ? {
              insideReceiverSegment: [
                transform(door.node, door.insideReceiverSegment[0]),
                transform(door.node, door.insideReceiverSegment[1]),
              ] as [Vec3, Vec3],
            }
          : {}),
        middle: project(transform(door.node, door.middle)),
        worldMiddle: transform(door.node, door.middle),
        polygon: door.polygon.length
          ? ring(
              door.polygon.map((p) => project(transform(door.node, [...p, door.outside[2]]))),
              `${placement.id}/${door.id} click polygon`,
            )
          : [],
      });
    for (const door of gameplay.doors) placeDoor(door);
    for (const lift of gameplay.lifts ?? []) {
      const id = `${placement.id}/${lift.id}`;
      const origin = transform(lift.node, [0, 0, 0]);
      const direction = transform(lift.node, [...lift.direction, 0]);
      const angle = Math.atan2(direction[0] - origin[0], origin[1] - direction[1]);
      lifts.push({
        id,
        type: lift.type,
        direction: (Math.round((angle * 8) / Math.PI) + 16) % 16,
        joins: (lift.joins ?? []).map((p) => transform(lift.node, p)),
      });
      for (const door of lift.doors) placeDoor(door, id);
    }
    for (const interior of gameplay.interiors ?? []) {
      const id = `${placement.id}/${interior.id}`;
      const origin = transform(interior.node, [0, 0, 0]);
      placedInteriors.push({
        id,
        joins: (interior.joins ?? []).map((join) => {
          const direction = transform(interior.node, [...join.direction, 0]);
          return {
            point: transform(interior.node, join.point),
            direction: [direction[0] - origin[0], direction[1] - origin[1]],
          };
        }),
      });
      for (const door of interior.doors) placeDoor(door, undefined, id);
    }
  }
  // Placed floors own their coverage. Cut it out of authored ground at the
  // same height so door endpoints never resolve to two overlapping areas.
  if (surfaces.some((s) => s.acceptsNavigationJoins)) {
    const placed = surfaces.filter((s) => !s.acceptsNavigationJoins);
    const ground = surfaces.filter((s) => s.acceptsNavigationJoins);
    const replacement: typeof surfaces = [];
    for (const surface of ground) {
      const cuts = placed
        .filter((other) => other.plane.every((n, i) => Math.abs(n - surface.plane[i]!) < 1e-7))
        .map((other) => [other.polygon, ...other.holes] as Polygon);
      const shape: Polygon = [surface.polygon, ...surface.holes];
      const remaining = cuts.length ? fixedPolygonBoolean("difference", shape, cuts) : [shape];
      for (const polygon of remaining)
        replacement.push({
          ...surface,
          // Changed ground becomes generated navigation. Its previous contour
          // labels no longer describe the clipped footprint.
          preserveMovementBoundary: cuts.length ? false : surface.preserveMovementBoundary,
          holeContours: cuts.length ? undefined : surface.holeContours,
          // Boolean clipping creates fractional remnants even when the input
          // terrain was quantized. Retain them until navigation union/rounding.
          polygon: ring(polygon[0]!, "Clipped terrain surface", 1e-8),
          worldPolygon: polygon[0]!.map(([x, y]): Point => [
            x,
            y + planeHeight(surface.plane, [x, y]),
          ]),
          worldHoles: polygon
            .slice(1)
            .map((hole) =>
              hole.map(([x, y]): Point => [x, y + planeHeight(surface.plane, [x, y])]),
            ),
          holes: polygon.slice(1).map((h) => ring(h, "Clipped terrain hole", 1e-8)),
        });
    }
    surfaces.splice(0, surfaces.length, ...placed, ...replacement);
    // Explicit exterior sockets can attach directly to the ground beside them.
    // Ordinary door boundaries remain separate; no locked doorway is bypassed.
    for (const join of assembleNavigationJoins(navigationJoins).unmatched) {
      const [a, b] = join.edge,
        dx = b[0] - a[0],
        dy = b[1] - b[2] - (a[1] - a[2]);
      const length = Math.hypot(dx, dy);
      const probe: Point = [
        (a[0] + b[0]) / 2 + (dy / length) * 0.5,
        (a[1] - a[2] + b[1] - b[2]) / 2 - (dx / length) * 0.5,
      ];
      const groundSurface = replacement.find(
        (s) =>
          // A slope changes height beside the socket. Compare both ends on
          // the shared edge; the outward probe only selects adjacent ground.
          join.edge.every(([x, y, z]) => Math.abs(planeHeight(s.plane, [x, y - z]) - z) < 1e-4) &&
          inside(probe, s.polygon) &&
          !s.holes.some((h) => inside(probe, h)) &&
          !movementBlockers.some(
            (blocker) =>
              blocker.owner === s.owner &&
              Math.abs(planeHeight(blocker.plane, probe) - planeHeight(s.plane, probe)) < 1e-4 &&
              inside(probe, blocker.polygon),
          ),
      );
      if (groundSurface?.navigationRegion)
        navigationJoins.push({
          owner: groundSurface.owner,
          region: groundSurface.navigationRegion,
          edge: [b, a],
        });
    }
  }
  options.onProgress?.("Connecting asset interiors and traversal");
  const connections: { from: string; to: string }[] = [];
  const availableInteriors = new Set(placedInteriors.map((interior) => interior.id));
  if (document.interiorConnections)
    validateInteriorConnections(document.interiorConnections, document.objects);
  for (const connection of document.interiorConnections ?? []) {
    const from = interiorEndpointId(connection.from),
      to = interiorEndpointId(connection.to);
    if (!availableInteriors.has(from) || !availableInteriors.has(to)) {
      const message = `Interior connection ${connection.id}: endpoint room is missing, hidden or has no compiled gameplay (${from} → ${to})`;
      if (!options.bestEffort) throw new Error(message);
      warnings.push(`${message}; connection omitted.`);
      continue;
    }
    connections.push({ from, to });
  }
  const interiorIdentities = assembleInteriors(placedInteriors, connections);
  for (const door of doors)
    if (door.interior) door.interior = interiorIdentities.get(door.interior)!;
  // A disconnected passage with no entrance needs no runtime room.
  let interiors = [...new Set(interiorIdentities.values())].filter((id) =>
    doors.some((door) => door.interior === id),
  );
  let assembledLifts: ReturnType<typeof assembleLiftSegments>;
  try {
    assembledLifts = assembleLiftSegments(lifts);
  } catch (error) {
    if (!(error instanceof UnavailableLiftJoin) || !options.bestEffort) throw error;
    throw new UnavailableLiftPlacement(error.segments[0]!, error.message, false, error.segments);
  }
  const assembledNavigation = assembleNavigationJoins(navigationJoins);
  for (const surface of [...surfaces, ...projectionSupports])
    if (surface.navigationRegion)
      surface.navigationRegion =
        assembledNavigation.identities.get(surface.navigationRegion) ?? surface.navigationRegion;
  for (const join of assembledNavigation.unmatched)
    warnings.push(
      `Navigation region ${join.region}: no matching boundary edge after placement; region remains independent.`,
    );
  lifts = assembledLifts.lifts;
  for (const surface of surfaces)
    if (surface.lift) surface.lift = assembledLifts.identities.get(surface.lift)!;
  for (const support of projectionSupports)
    if (support.lift) support.lift = assembledLifts.identities.get(support.lift)!;
  for (const door of doors) if (door.lift) door.lift = assembledLifts.identities.get(door.lift)!;
  if (!surfaces.length && !options.bestEffort && !omitted.size)
    throw new Error(
      "Assets define no walkable surfaces; a map rectangle is not a substitute for authored ground",
    );
  if (!surfaces.length)
    warnings.push(
      "No authored walkable surfaces are available. This export has no traversable ground.",
    );
  // Coordinates are projected pixels after subtracting the image origin.
  // Cropping affects compiled navigation only; the editor keeps all authored data.
  const frame = polygon([
    [0, 0],
    [bounds[2], 0],
    [bounds[2], bounds[3]],
    [0, bounds[3]],
  ]);
  const outsideFrame = ([x, y]: Point) => x < 0 || y < 0 || x > bounds[2] || y > bounds[3];
  const outsideAnchor = ([x, y, z]: Vec3) =>
    x < 0 || y - z < 0 || x >= bounds[2] || y - z >= bounds[3];
  const cropped = surfaces.some((surface) => surface.polygon.some(outsideFrame));
  if (cropped)
    warnings.push(
      "Gameplay navigation is clipped to the export frame; out-of-bounds editor content is retained.",
    );
  // Material records are referenced by receivers and by ground queries. Rebuild
  // every reference after clipping, since one concave region can become islands.
  const materialRemap = new Map<number, number[]>();
  const clippedMaterials: typeof materials = [];
  for (const [index, material] of materials.entries()) {
    const regions = material.polygon.points.some(outsideFrame)
      ? fixedPolygonBoolean("intersection", polygon(material.polygon.points), [frame])
      : [[material.polygon.points]];
    const indices: number[] = [];
    for (const region of regions) {
      const points = simplifyMotionRing(
        region[0]!.map(([x, y]): Point => [quantize(x), quantize(y)]),
      );
      if (points.length < 3 || Math.abs(signedArea(points)) < 0.5) continue;
      if (clippedMaterials.length > 65535) throw new Error("Too many clipped material regions");
      indices.push(clippedMaterials.length);
      clippedMaterials.push({ ...material, polygon: { points } });
    }
    materialRemap.set(index, indices);
  }
  const remapMaterials = (indices: number[]) =>
    indices.flatMap((index) => materialRemap.get(index)!);
  for (const obstacle of sight)
    obstacle.material_indices = remapMaterials(obstacle.material_indices);
  for (const support of projectionSupports)
    support.materialIndices = remapMaterials(support.materialIndices);
  groundMaterials.splice(0, groundMaterials.length, ...remapMaterials(groundMaterials));
  materials.splice(0, materials.length, ...clippedMaterials);
  options.onProgress?.("Constructing walkable geometry");
  const planes: HeightPlane[] = [];
  const planeBuckets = new Map<string, HeightPlane[]>();
  const canonicalPlanes = new Map<HeightPlane, HeightPlane>();
  const planeSurfaces = new Map<HeightPlane, typeof surfaces>();
  for (const surface of surfaces.filter((s) => !s.lift)) {
    const bucket = surface.plane.map((n) => Math.floor(n / 1e-7));
    let canonical: HeightPlane | undefined;
    // Neighbor buckets retain the existing tolerance at bucket boundaries.
    for (let x = -1; x <= 1 && !canonical; x++)
      for (let y = -1; y <= 1 && !canonical; y++)
        for (let z = -1; z <= 1 && !canonical; z++)
          canonical = planeBuckets
            .get(`${bucket[0]! + x},${bucket[1]! + y},${bucket[2]! + z}`)
            ?.find((p) => p.every((n, i) => Math.abs(n - surface.plane[i]!) < 1e-7));
    if (!canonical) {
      canonical = surface.plane;
      planes.push(canonical);
      const key = bucket.join(",");
      const members = planeBuckets.get(key) ?? [];
      members.push(canonical);
      planeBuckets.set(key, members);
      planeSurfaces.set(canonical, []);
    }
    canonicalPlanes.set(surface.plane, canonical);
    planeSurfaces.get(canonical)!.push(surface);
  }
  const planeSupports = new Map<HeightPlane, typeof projectionSupports>();
  for (const support of projectionSupports) {
    const canonical = canonicalPlanes.get(support.plane) ?? support.plane;
    const members = planeSupports.get(canonical) ?? [];
    members.push(support);
    planeSupports.set(canonical, members);
  }
  planes.sort((a, b) => a[2] - b[2] || a[0] - b[0] || a[1] - b[1]);
  // Ordinary surfaces occupy conventional layers; all lifts use the reserved last layer.
  const layers: CompiledAssetGeometry["motion_data"]["layers"] = Array.from(
    { length: Math.max(1, planes.length) + 1 },
    () => [],
  );
  const groups = planes.flatMap((plane, layer) => {
    const matching = planeSurfaces.get(plane)!;
    return [...new Set(matching.map((s) => s.navigationRegion))].map((region) => ({
      plane,
      layer,
      lift: undefined as string | undefined,
      navigationRegion: region,
      surfaces: matching.filter((s) => s.navigationRegion === region),
    }));
  });
  for (const surface of surfaces.filter((s) => s.lift))
    groups.push({
      plane: surface.plane,
      layer: layers.length - 1,
      lift: surface.lift,
      navigationRegion: undefined,
      surfaces: [surface],
    });
  const areas: (NavigationAnchorArea & {
    physical?: NavigationAnchorArea;
    lift?: string;
    navigationRegion?: string;
    sector: number;
    layer: number;
  })[] = [];
  const jumpWalkAreas: JumpWalkArea[] = [];
  let sector = 0;
  const navigationPieces: NavigationPiece[] = [];
  const boundsOf = (points: Point[]): [number, number, number, number] => {
    const bounds: [number, number, number, number] = [Infinity, Infinity, -Infinity, -Infinity];
    for (const [x, y] of points) {
      bounds[0] = Math.min(bounds[0], x);
      bounds[1] = Math.min(bounds[1], y);
      bounds[2] = Math.max(bounds[2], x);
      bounds[3] = Math.max(bounds[3], y);
    }
    return bounds;
  };
  const supportBounds = new Map(
    projectionSupports.map((support) => [support, boundsOf(support.polygon)]),
  );
  const solidGeometry = movementSolids
    .filter(({ shape }) => shape.solid)
    .map(({ owner, shape, headroom = 0 }) => {
      const footprint = shape.points.map((p): Point => [p.x, p.y]);
      return {
        owner,
        footprint,
        bounds: boundsOf(footprint),
        top: heightPlane(
          shape.points.map((p) => [p.x, p.y, p.z_top]),
          false,
        ),
        bottom: heightPlane(
          shape.points.map((p) => [p.x, p.y, p.z_bottom - headroom]),
          false,
        ),
      };
    });
  const navigationPlaneCounts = new Map<string, number>();
  const clearanceKey = (s: (typeof surfaces)[number]) =>
    JSON.stringify([s.owner, s.plane, s.polygon, s.holes]);
  const completeClearances = new Set(movementClearances.map(clearanceKey));
  for (const group of groups)
    if (group.navigationRegion)
      navigationPlaneCounts.set(
        group.navigationRegion,
        (navigationPlaneCounts.get(group.navigationRegion) ?? 0) + 1,
      );
  for (const { layer, plane, lift, navigationRegion, surfaces: group } of groups) {
    const joinedWall =
      navigationRegion !== undefined &&
      (navigationPlaneCounts.get(navigationRegion) ?? 0) > 1 &&
      group.every((s) => s.owner.startsWith("wall-spline-"));
    // Cropped boundaries become generated polygons: preserved contours can extend
    // beyond the frame and must not authorize movement into the invisible area.
    const preserve =
      group.some((s) => s.preserveMovementBoundary) &&
      !(group.length > 1 && group.some((s) => s.acceptsNavigationJoins)) &&
      !group.some((s) => s.polygon.some(outsideFrame));
    if (preserve && (group.length !== 1 || lift))
      throw new Error(
        "Preserved movement boundary needs one ordinary surface per navigation region",
      );
    const cutouts: Polygon[] = preserve ? group[0]!.holes.map((h) => polygon(h)) : [];
    const contourGroups: (string | undefined)[] = cutouts.map(
      (_, index) => group[0]!.holeContours?.[index],
    );
    const input = group.map((s): Polygon => [
      polygon(s.polygon)[0]!,
      ...s.holes.map((h) => polygon(h)[0]!),
    ]);
    let merged = preserve ? [] : unionMovementSurfaces(input, `Movement layer ${layer}`, warnings);
    // Authored movement exclusions belong to a plane and follow their asset placement.
    // Subtract whole polygons so holes in blockers remain walkable islands.
    for (const blocker of movementBlockers) {
      if (!plane.every((n, i) => Math.abs(n - blocker.plane[i]!) < 1e-7)) continue;
      if (preserve) {
        cutouts.push([polygon(blocker.polygon)[0]!, ...blocker.holes.map((h) => polygon(h)[0]!)]);
        contourGroups.push(blocker.movementContour);
        continue;
      }
      merged = fixedPolygonBoolean("difference", merged, [
        [polygon(blocker.polygon)[0]!, ...blocker.holes.map((h) => polygon(h)[0]!)],
      ]);
    }
    // Intersect solids with this surface's plane in world XY, then project
    // the resulting slice. Bounding-box clipping also handles concave solids.
    // Retain the plane fitted to placed 3D vertices. Refitting a world plane
    // from an already projected/rounded outline amplifies projection error.
    const worldPlane = group[0]!.worldPlane;
    const worldBounds = boundsOf(
      group.flatMap((surface) => [
        ...surface.worldPolygon,
        // The emitted integer outline can extend slightly past the authored
        // floor. Include it in broad-phase bounds so nearby solids still cut it.
        ...surface.polygon.map(([x, y]): Point => [x, y + planeHeight(plane, [x, y])]),
      ]),
    );
    const wallCuts: Polygon[] = [];
    // An explicit clearance identical to the whole deck already excludes these
    // support solids. Avoid subtracting and re-adding their nearly coincident
    // caps, while retaining all cuts from separately placed objects.
    const clearedOwner =
      joinedWall &&
      group.every((s) => s.owner === group[0]!.owner && completeClearances.has(clearanceKey(s)))
        ? group[0]!.owner
        : undefined;
    for (const { owner, footprint, bounds: solidBounds, top, bottom } of solidGeometry) {
      if (owner === clearedOwner) continue;
      // Most grid triangles are far from most placed solids. Their disjoint
      // world-space bounds exclude intersection before any polygon operations.
      if (
        solidBounds[2] < worldBounds[0] ||
        solidBounds[0] > worldBounds[2] ||
        solidBounds[3] < worldBounds[1] ||
        solidBounds[1] > worldBounds[3]
      )
        continue;
      const slice = movementVolumeHeightSlice(footprint, worldPlane, bottom, top);
      if (slice.length < 3 || Math.abs(signedArea(slice)) < 1e-7) continue;
      const cuts = fixedPolygonBoolean("intersection", polygon(footprint), [polygon(slice)]);
      for (const cut of cuts) {
        const projected: Polygon = cut.map((r) =>
          r.map(([x, y]) => [x, y - planeHeight(worldPlane, [x, y])]),
        );
        let regions = [projected];
        for (const clearance of movementClearances) {
          if (
            clearance.owner !== owner ||
            !clearance.polygon.every(
              (point) =>
                Math.abs(planeHeight(plane, point) - planeHeight(clearance.plane, point)) <=
                MOVEMENT_CONTACT_TOLERANCE,
            )
          )
            continue;
          regions = fixedPolygonBoolean("difference", regions, [
            [polygon(clearance.polygon)[0]!, ...clearance.holes.map((h) => polygon(h)[0]!)],
          ]);
        }
        if (preserve) {
          cutouts.push(...regions);
          contourGroups.push(...regions.map(() => undefined));
        } else if (regions.length) {
          if (owner.startsWith("wall-spline-")) wallCuts.push(...regions);
          else merged = subtractMovementCollision(merged, regions);
        }
      }
    }
    // Join neighboring wall triangles before rounding their boundary. Rounding
    // each subtraction separately can turn shared diagonals into walkable slivers.
    if (wallCuts.length && joinedWall) {
      const joined = unionMovementSurfaces(wallCuts, `Wall cuts on layer ${layer}`, warnings);
      merged = fixedPolygonBoolean("difference", merged, [joined], 1024);
    } else if (wallCuts.length) {
      const joined = unionMovementSurfaces(wallCuts, `Wall cuts on layer ${layer}`, warnings);
      // Subpixel cracks from snapped T-junctions cannot represent walkable holes.
      // Remove these before grid rounding can inflate them into narrow islands.
      let sealedHoles = 0;
      const seal = (regions: Polygon[]) =>
        regions.map((region) =>
          region.filter((ring, index) => {
            if (!index) return true;
            const retain = !narrowMovementRing(ring);
            if (!retain) sealedHoles++;
            return retain;
          }),
        );
      const rounded = normalizeGeneratedMotion(
        seal(fixedPolygonBoolean("intersection", joined, [merged], 1024)),
        `Wall cuts on layer ${layer}`,
        warnings,
      );
      merged = fixedPolygonBoolean("difference", merged, [seal(rounded)], 1);
      if (sealedHoles)
        warnings.push(
          `Movement layer ${layer}: sealed ${sealedHoles} wall-cut holes too narrow for integer-grid navigation.`,
        );
    }
    if (preserve) {
      navigationPieces.push({
        layer,
        plane,
        navigationRegion,
        preserveMovementBoundary: true,
        worldPlane,
        receivingPolygon: ring(group[0]!.polygon, "Preserved receiving boundary"),
        ...preserveMovementBoundary(group[0]!.polygon, cutouts, warnings, contourGroups),
      });
      continue;
    }
    if (group.some((s) => s.polygon.some(outsideFrame)))
      merged = fixedPolygonBoolean("intersection", merged, [frame]);
    if (joinedWall) {
      // A deformed deck spans many small receiving planes. Round its navigation
      // only after joining those planes, so narrow triangle tips cannot become gaps.
      for (const polygon of merged)
        navigationPieces.push({
          layer,
          plane,
          navigationRegion,
          closeDeformationSeams: true,
          worldPlane,
          polygon: ring(polygon[0]!, "Wall navigation piece", 1e-7),
          blockers: polygon.slice(1).map((h) => ring(h, "Wall navigation hole", 1e-7)),
        });
      continue;
    }
    const normalized = walls.descriptors.length
      ? normalizeGeneratedMotion(merged, `Movement layer ${layer}`, warnings)
      : merged.flatMap((region) =>
          normalizeGeneratedMotion([region], `Movement layer ${layer}`, warnings),
        );
    const preciseHoles = indexPreciseBlockers(merged.flatMap((candidate) => candidate.slice(1)));
    let blockedCoverage: MultiPolygon | undefined;
    for (const poly of normalized) {
      if (wallCuts.length && narrowMovementRing(poly[0]!)) {
        warnings.push(
          `Movement layer ${layer}: omitted a wall-cut fragment too narrow for integer-grid navigation.`,
        );
        continue;
      }
      const quantized = quantizeGeneratedMotionPolygon(
        poly,
        quantize,
        `Movement layer ${layer}`,
        warnings,
      );
      if (!quantized) continue;
      const boundary = ring(quantized[0]!, `Merged movement boundary on layer ${layer}`);
      const receivingCandidates = merged.filter((candidate) => {
        const rounded = quantizeGeneratedMotionPolygon(
          candidate,
          quantize,
          "Receiving boundary",
          [],
        );
        return rounded && polygonClipping.xor([rounded[0]!], [boundary]).length === 0;
      });
      const blockers = quantized
        .slice(1)
        .map((r) => ring(r, `Merged movement hole on layer ${layer}`));
      const isLanding =
        !lift &&
        doors.some(
          (door) =>
            door.lift &&
            containsNavigationAnchor({ plane, polygon: boundary, blockers }, door.outsideAnchor, {
              allowBlocked: true,
            }),
        );
      const receivingBoundary =
        receivingCandidates.length === 1
          ? ring(receivingCandidates[0]![0]!, "Receiving boundary", 1e-8)
          : !lift &&
              doors.some(
                (door) =>
                  door.lift &&
                  containsNavigationAnchor(
                    { plane, polygon: boundary, blockers },
                    door.outsideAnchor,
                    { allowBlocked: true },
                  ),
              )
            ? restoreReceivingBoundary(boundary, merged)
            : undefined;
      navigationPieces.push({
        layer,
        plane,
        worldPlane,
        lift,
        navigationRegion,
        polygon: boundary,
        ...(receivingBoundary ? { receivingPolygon: receivingBoundary } : {}),
        preciseBlockers: blockers.flatMap((points) => {
          const matching = (preciseHoles.get(motionBoundsKey(points)) ?? [])
            .filter(({ rounded }) => polygonClipping.xor([rounded], [points]).length === 0)
            .map(({ exact }) => exact);
          if (matching.length) return matching;
          if (!isLanding) return [];
          blockedCoverage ??= fixedPolygonBoolean("difference", [frame], [merged]);
          const restored = restoreObstacleBoundary(points, blockedCoverage);
          return restored ? [restored] : [];
        }),
        blockers,
      });
    }
  }
  const physicalStairs = new Map<string, ReturnType<typeof compilePhysicalStairRegion>>();
  const compilePhysicalLift = (lift: (typeof lifts)[number]) => {
    const floor = surfaces.filter((surface) => surface.lift === lift.id);
    const worldRing = (points: Point[], plane: HeightPlane): Vec3[] =>
      points.map(([x, y]) => [x, y, planeHeight(plane, [x, y])]);
    const compiled = compilePhysicalStairRegion({
      prepareWalkingClearance: lift.type === 1,
      frame: [0, 0, bounds[2], bounds[3]],
      surfaces: floor.map((surface) => ({
        polygon: worldRing(surface.worldPolygon, surface.worldPlane),
        holes: surface.worldHoles.map((hole) => worldRing(hole, surface.worldPlane)),
      })),
      doors: doors
        .filter((door) => door.lift === lift.id)
        .map((door) => ({
          inside: door.inside,
          middle: door.worldMiddle,
          outside: door.outside,
        })),
      solids: solidGeometry.map((solid) => ({
        owner: solid.owner,
        polygon: solid.footprint,
        holes: [],
        top: solid.top,
        bottom: solid.bottom,
      })),
      clearances: movementClearances.map((surface) => ({
        owner: surface.owner,
        polygon: surface.worldPolygon,
        holes: surface.worldHoles,
        plane: surface.worldPlane,
      })),
      blockers: [
        ...transitionBlockers.map((blocker) => ({
          ...blocker,
          plane: blocker.worldPlane,
          polygon: blocker.worldPolygon,
          holes: blocker.worldHoles,
        })),
        ...movementBlockers.map((blocker) => ({
          transition: blocker.owner,
          fixed: true,
          applied: false,
          plane: blocker.worldPlane,
          polygon: blocker.worldPolygon,
          holes: blocker.worldHoles,
        })),
      ],
    });
    return compiled;
  };
  options.onProgress?.("Connecting navigation regions");
  let navigationRegions: ReturnType<typeof assembleNavigationRegions>;
  for (;;) {
    try {
      navigationRegions = assembleNavigationRegions(navigationPieces, warnings);
      break;
    } catch (error) {
      if (!(error instanceof DisconnectedLiftRegion)) throw error;
      const lift = lifts.find((lift) => lift.id === error.lift);
      let physical: ReturnType<typeof compilePhysicalStairRegion> | undefined;
      if (lift) {
        try {
          const candidate = compilePhysicalLift(lift);
          const permanent = candidate.navigation.obstacles.filter(
            (obstacle) => candidate.area.obstacles[obstacle.motion_obstacle]!.state_id === 0,
          );
          const free = polygonClipping.difference(
            [candidate.navigation.boundary],
            permanent.map((obstacle) => [obstacle.polygon]),
          );
          // A real collision cut must still disconnect a lift. Only a valid,
          // connected physical floor can replace a pinched screen projection.
          if (free.length === 1) physical = candidate;
        } catch (physicalError) {
          warnings.push(
            `Lift ${error.lift}: physical connectivity unavailable: ${String(physicalError)}`,
          );
          physical = undefined;
        }
      }
      if (physical) {
        physicalStairs.set(error.lift, physical);
        const members = navigationPieces.filter((piece) => piece.lift === error.lift);
        for (let i = navigationPieces.length - 1; i >= 0; i--)
          if (navigationPieces[i]!.lift === error.lift) navigationPieces.splice(i, 1);
        navigationPieces.push({
          ...members[0]!,
          polygon: physical.area.polygon.points,
          blockers: [],
        });
        continue;
      }
      if (!options.bestEffort) throw error;
      const segments = [...assembledLifts.identities]
        .filter(([, lift]) => lift === error.lift)
        .map(([id]) => id);
      if (!segments.length) throw error;
      throw new UnavailableLiftPlacement(error.lift, error.message, false, segments);
    }
  }
  for (const region of navigationRegions)
    if (region.pieces.every((p) => p.navigationRegion?.startsWith("wall-spline-"))) {
      const count = region.blockers.length;
      region.blockers = region.blockers.filter((h) => !narrowMovementRing(h));
      if (region.blockers.length < count)
        warnings.push(
          `Joined wall navigation: sealed ${count - region.blockers.length} holes too narrow for integer-grid navigation.`,
        );
    }
  allocateLightReceivingLayers(navigationRegions, lights, layers.length - 1, inside);
  const liftLayer = compactNavigationLayers(navigationRegions);
  // A raised receiving volume can supply the physical landing over a lower
  // navigation plane. Require an unambiguous ground binding and retain its
  // actual walkable boundary and holes when checking the ladder approach.
  options.onProgress?.("Constructing receiving surfaces");
  const receiverContours = new Map<
    NavigationPiece,
    Pick<NavigationPiece, "polygon" | "blockers">
  >();
  for (const region of navigationRegions)
    for (const piece of region.pieces) {
      const precise = indexPreciseBlockers(piece.preciseBlockers ?? []);
      receiverContours.set(piece, {
        polygon: piece.receivingPolygon ?? piece.polygon,
        blockers: piece.blockers.map((points) => {
          const candidates = (precise.get(motionBoundsKey(points)) ?? []).filter(
            ({ rounded }) => polygonClipping.xor([rounded], [points]).length === 0,
          );
          return candidates.length === 1 ? candidates[0]!.exact : points;
        }),
      });
    }
  options.onProgress?.("Connecting receiving landings");
  const receiverLandings = projectionReceivers.flatMap((receiver) => {
    if (receiver.receiverSegment) return [];
    const regions = navigationRegions.filter(
      (region) =>
        !region.lift &&
        region.pieces.some((piece) => containsNavigationAnchor(piece, receiver.anchor)),
    );
    if (regions.length !== 1) return [];
    const top = receiver.shape.points.map((p): Vec3 => [p.x, p.y - p.z_top, p.z_top]);
    return [
      {
        area: {
          plane: heightPlane(top.slice(0, 3)),
          polygon: top.map(([x, y]): Point => [x, y]),
          blockers: [],
        },
        region: regions[0]!,
      },
    ];
  });
  for (const lift of lifts.filter(
    (lift) => lift.type === 1 || lift.type === 2 || lift.type === 3,
  )) {
    if (physicalStairs.has(lift.id)) continue;
    try {
      if (lift.type === 2 || lift.type === 3)
        for (const door of doors.filter((door) => door.lift === lift.id)) {
          // Terrain triangles can support opposite ends of the same approach.
          const supported = [door.outside, door.worldMiddle].every((point) =>
            surfaces.some((surface) => {
              if (surface.lift) return false;
              const area = {
                coordinateSpace: "world" as const,
                plane: surface.worldPlane,
                polygon: surface.worldPolygon,
                blockers: surface.worldHoles,
              };
              return containsNavigationAnchor(
                {
                  ...area,
                  // A climb can enter at the edge of a platform opening.
                  // Only its seam may touch that edge; the outside point must
                  // still have ordinary support and hole interiors stay blocked.
                  blockers:
                    point === door.outside
                      ? area.blockers
                      : area.blockers.filter(
                          (hole) =>
                            !onClippedReceivingBoundary(
                              [door.worldMiddle[0], door.worldMiddle[1]],
                              hole,
                            ),
                        ),
                },
                point,
              );
            }),
          );
          const receiverSupported = receiverLandings.some(({ area, region }) =>
            [door.outside, door.worldMiddle].every(
              (point) =>
                containsNavigationAnchor(area, point) &&
                region.pieces.some((piece) => {
                  const projected: Point = [point[0], point[1] - point[2]];
                  const contour = receiverContours.get(piece)!;
                  return (
                    (inside(projected, contour.polygon, true) ||
                      onClippedReceivingBoundary(projected, contour.polygon)) &&
                    !contour.blockers.some(
                      (hole) =>
                        inside(projected, hole, true) &&
                        !(
                          point === door.worldMiddle && onClippedReceivingBoundary(projected, hole)
                        ),
                    )
                  );
                }),
            ),
          );
          if (!supported && !receiverSupported)
            throw new Error(
              `Landing does not reach physical ${lift.type === 2 ? "ladder" : "wall"} door ${door.name}`,
            );
        }
      physicalStairs.set(lift.id, compilePhysicalLift(lift));
    } catch (error) {
      warnings.push(
        `Lift ${lift.id}: physical navigation unavailable; retaining projected navigation: ${error instanceof Error ? error.message : String(error)}`,
      );
    }
  }
  // Receiving planes can share a navigation region; their provisional layers are not runtime layers.
  layers.length = 0;
  while (layers.length <= liftLayer) layers.push([]);
  let receivingRegion = 0;
  for (const region of navigationRegions) {
    receivingRegion += 1;
    options.onProgress?.(
      `Constructing receiving boundaries (${receivingRegion}/${navigationRegions.length})`,
    );
    const { layer, lift, polygon: boundary, blockers, pieces } = region;
    const plane = pieces[0]!.plane;
    const physical = lift ? physicalStairs.get(lift) : undefined;
    const changing = physical
      ? {
          pairs: physical.pairs,
          obstacles: physical.area.obstacles,
          initial: physical.initialBlockers,
        }
      : compileTransitionObstacles(
          boundary,
          blockers,
          plane,
          transitionBlockers,
          warnings,
          pieces.length > 1 ? pieces : undefined,
          pieces[0]!.preserveMovementBoundary === true,
          pieces[0]!.worldPlane,
        );
    for (const [id, pair] of changing.pairs)
      transitions
        .find((t) => t.id === id)!
        .changes.push({
          layer,
          sector,
          changing_obstacle: pair,
        });
    const preciseBlockers = indexPreciseBlockers(
      pieces.flatMap((piece) => piece.preciseBlockers ?? []),
    );
    const raisedLanding = receiverLandings.some(
      ({ area, region: landing }) =>
        landing === region &&
        doors.some(
          (door) =>
            door.lift &&
            physicalStairs.has(door.lift) &&
            containsNavigationAnchor(area, door.outside),
        ),
    );
    // Ordinary receiving floors also need their pre-grid edges: rounding can
    // trim the height-matched seam between a doorstep and adjoining terrain.
    const physicalLanding =
      (!lift && pieces.some((piece) => piece.receivingPolygon !== undefined)) ||
      raisedLanding ||
      (!lift &&
        doors.some(
          (door) =>
            door.lift &&
            physicalStairs.has(door.lift) &&
            pieces.some((piece) => containsNavigationAnchor(piece, door.outsideAnchor)),
        ));
    let joinedBlockedCoverage: MultiPolygon | undefined;
    const recoverJoinedBlocker = (points: Point[]): Point[] | undefined => {
      if (!physicalLanding || pieces.length < 2) return undefined;
      if (!joinedBlockedCoverage) {
        joinedBlockedCoverage = recoverJoinedCoverage(pieces, frame);
      }
      return restoreObstacleBoundary(points, joinedBlockedCoverage);
    };
    const preciseBoundaries = physicalLanding
      ? (
          indexPreciseBlockers(
            pieces.flatMap((piece) => (piece.receivingPolygon ? [piece.receivingPolygon] : [])),
          ).get(motionBoundsKey(boundary)) ?? []
        ).filter(
          ({ exact, rounded }) =>
            exact.some((point) =>
              point.some((value) => Math.abs(value - Math.round(value)) > 2 / 1048576),
            ) && polygonClipping.xor([rounded], [boundary]).length === 0,
        )
      : [];
    const recoveredBoundary =
      preciseBoundaries.length === 1
        ? preciseBoundaries[0]!.exact
        : physicalLanding && pieces.length > 1
          ? joinedReceivingBoundary(pieces, boundary)
          : undefined;
    const preciseBoundary =
      recoveredBoundary && fixedPolygonBoolean("xor", [recoveredBoundary], [[boundary]]).length > 0
        ? recoveredBoundary
        : undefined;
    options.onProgress?.(
      `Constructing receiving obstacles (${receivingRegion}/${navigationRegions.length})`,
    );
    layers[layer]!.push(
      physical?.area ?? {
        is_lift: !!lift,
        state_id: 0,
        polygon: { points: boundary },
        ...(preciseBoundary ? { precise_polygon: preciseBoundary } : {}),
        skeleton_segments: [],
        flags: 0,
        obstacles: [
          ...blockers.map((points) => {
            const candidates = (preciseBlockers.get(motionBoundsKey(points)) ?? []).filter(
              ({ rounded }) => polygonClipping.xor([rounded], [points]).length === 0,
            );
            const exact =
              candidates.length === 1
                ? candidates[0]!.exact
                : candidates.length === 0
                  ? recoverJoinedBlocker(points)
                  : undefined;
            return {
              state_id: 0,
              polygon: { points },
              ...(exact ? { precise_polygon: exact } : {}),
            };
          }),
          ...changing.obstacles,
        ],
      },
    );
    // Projection surfaces provide layer-aware elevation and picking.
    let receivingPiece = 0;
    for (const piece of pieces) {
      if (receivingPiece % 64 === 0)
        options.onProgress?.(
          `Constructing receiving materials (${receivingRegion}/${navigationRegions.length}, ${receivingPiece + 1}/${pieces.length})`,
        );
      receivingPiece += 1;
      areas.push({
        ...piece,
        sector,
        layer,
        blockers: physical
          ? physical.area.obstacles
              .filter(
                (obstacle) => obstacle.state_id === 0 || (obstacle.state_id & 0x55555555) !== 0,
              )
              .map((obstacle) => obstacle.polygon.points)
          : [...piece.blockers, ...changing.initial],
        ...(physical
          ? {
              physical: {
                coordinateSpace: "world" as const,
                plane: physical.navigation.plane,
                floorPatches: physical.navigation.floor_patches,
                polygon: physical.navigation.boundary,
                blockers: physical.initialBlockers,
              },
            }
          : {}),
      });
      jumpWalkAreas.push({
        plane: piece.plane,
        polygon: boundary,
        blockers: physical
          ? physical.area.obstacles.map((o) => o.polygon.points)
          : [...blockers, ...changing.obstacles.map((o) => o.polygon.points)],
      });
      const walkableCoverage =
        piece.preserveMovementBoundary && piece.blockers.length
          ? fixedPolygonBoolean(
              "difference",
              [piece.polygon],
              piece.blockers.map((b) => [b]),
            )
          : [[polygon(piece.polygon)[0]!, ...piece.blockers.map((h) => polygon(h)[0]!)]];
      const receivingBounds = boundsOf(walkableCoverage.flatMap((polygon) => polygon[0]!));
      const supports = (planeSupports.get(piece.plane) ?? []).filter((support) => {
        const box = supportBounds.get(support)!;
        return (
          support.lift === piece.lift &&
          support.navigationRegion === piece.navigationRegion &&
          support.plane.every((n, i) => Math.abs(n - piece.plane[i]!) < 1e-7) &&
          // Distant supports cannot overlap this floor. Keep touching bounds
          // for the exact fixed-point intersection below.
          box[0] <= receivingBounds[2] &&
          box[2] >= receivingBounds[0] &&
          box[1] <= receivingBounds[3] &&
          box[3] >= receivingBounds[1] &&
          // Receiving footprints may extend into blocked space, but ownership
          // comes from walkable coverage, excluding separate islands in holes.
          fixedPolygonBoolean(
            "intersection",
            [polygon(support.polygon)[0]!, ...support.holes.map((h) => polygon(h)[0]!)],
            walkableCoverage,
          ).length > 0
        );
      });
      for (const support of supports) {
        if (support.obstacleIndex === undefined) continue;
        const receiver = sight[support.obstacleIndex]!;
        if (
          Array.isArray(receiver.projection_area) &&
          (receiver.projection_area[0] !== sector || receiver.projection_area[1] !== layer)
        )
          throw new Error(
            `Projection volume ${support.owner}/${support.obstacleIndex} spans multiple receiving areas (${receiver.projection_area.join(":")} and ${sector}:${layer}); author its navigation region before export`,
          );
        receiver.projection_area = [sector, layer];
      }
      const receivingPolygon = !lift ? (piece.receivingPolygon ?? piece.polygon) : piece.polygon;
      const preciseLanding =
        !lift &&
        fixedPolygonBoolean("xor", polygon(receivingPolygon), [polygon(piece.polygon)]).length > 0;
      for (const material of partitionProjectionMaterials(receivingPolygon, supports, warnings)) {
        if (material.obstacleIndex !== undefined) continue;
        if (
          !lift &&
          !preciseLanding &&
          !material.explicit &&
          !piece.plane.some((n) => Math.abs(n) > 1e-7)
        )
          continue;
        const receivingPlane = material.planePoints
          ? heightPlane(material.planePoints.map(([x, y, z]) => [x, y - z, z]))
          : piece.plane;
        const points = material.polygon.map(([x, y]) => {
          const height = planeHeight(receivingPlane, [x, y]);
          return { x, y: y + height, z_bottom: height, z_top: height };
        });
        for (const nativePoints of nativeReceiverGeometry(
          points,
          `Receiving material partition in ${sector}:${layer}`,
          warnings,
        ))
          sight.push({
            projection_plane:
              material.planePoints ?? projectionPlaneAnchors(material.polygon, receivingPlane),
            points: nativePoints,
            projection_area: [sector, layer],
            opaque: false,
            solid: false,
            mouse: true,
            show_shadow_polygon: false,
            default_material: material.defaultMaterial,
            material_indices: material.materialIndices,
          });
      }
    }
    sector +=
      1 + (physical ? physical.area.obstacles.length : blockers.length + changing.obstacles.length);
  }
  options.onProgress?.("Binding receiving surfaces");
  for (const support of projectionSupports)
    if (
      support.obstacleIndex !== undefined &&
      !sight[support.obstacleIndex]!.projection_area &&
      fixedPolygonBoolean("intersection", polygon(support.polygon), [frame]).length
    )
      throw new Error("Projection volume has no compiled receiving area");
  class UnresolvedSurface extends Error {}
  class OutsideExportFrame extends UnresolvedSurface {}
  const resolve = (
    point: Vec3,
    label: string,
    lift?: string | null,
    allowBlocked = false,
    projected: Point = project(point),
  ) => {
    if (
      cropped &&
      (outsideFrame(projected) || projected[0] === bounds[2] || projected[1] === bounds[3])
    )
      throw new OutsideExportFrame(`${label}: anchor lies outside the export frame`);
    const matches = areas.filter(
      (a) =>
        (lift === null || a.lift === lift) &&
        containsNavigationAnchor(a.physical ?? a, point, { projected, allowBlocked }),
    );
    if (new Set(matches.map((a) => a.sector)).size !== 1) {
      const containing = areas.filter((a) =>
        containsNavigationAnchor(a.physical ?? a, point, {
          projected,
          allowBlocked: true,
          requireHeight: false,
        }),
      );
      const details = containing.slice(0, 8).map((a) => ({
        sector: a.sector,
        layer: a.layer,
        lift: a.lift,
        height: navigationAnchorHeight(a.physical ?? a, point),
        blocked: !containsNavigationAnchor(a.physical ?? a, point, {
          projected,
          requireHeight: false,
        }),
      }));
      throw new UnresolvedSurface(
        `${label} must resolve to exactly one ${allowBlocked ? "" : "unblocked "}walkable surface (found ${matches.length}); world point ${JSON.stringify(point)}, projected ${JSON.stringify(projected)}; containing areas (${containing.length}, showing up to 8) ${JSON.stringify(details)}`,
      );
    }
    return matches[0]!;
  };
  const resolveReceivingSegment = (
    segment: [Vec3, Vec3],
    fallbackAnchor: Vec3,
    label: string,
    allowBlocked = false,
  ): Vec3 => {
    const matches = areas.flatMap((area) => {
      if (area.lift) return [];
      let point: Vec3 | undefined;
      try {
        point = lightReceiverIntersection(segment, area.plane, label);
      } catch (error) {
        if (!(error instanceof Error)) throw error;
        throw new UnresolvedSurface(error.message);
      }
      if (!point) return [];
      const projected = project(point);
      if (
        !inside(projected, area.polygon) ||
        (!allowBlocked && area.blockers.some((b) => inside(projected, b)))
      )
        return [];
      return [{ area, point }];
    });
    const first = matches[0];
    if (!first && cropped && outsideAnchor(fallbackAnchor))
      throw new OutsideExportFrame(
        `${label}: receiving segment has no surface inside the export frame`,
      );
    if (
      !first ||
      matches.some(
        ({ area, point }) =>
          area.sector !== first.area.sector ||
          point.some((v, i) => Math.abs(v - first.point[i]!) > 1e-4),
      )
    )
      throw new UnresolvedSurface(
        `${label}: receiving segment must intersect exactly one ${allowBlocked ? "" : "unblocked "}surface`,
      );
    return first.point;
  };
  const boundReceivers = projectionReceivers.flatMap((receiver) => {
    let area;
    try {
      const label = `${receiver.id} navigation anchor`;
      const anchor = receiver.receiverSegment
        ? resolveReceivingSegment(receiver.receiverSegment, receiver.anchor, label)
        : receiver.anchor;
      area = resolve(anchor, label);
    } catch (error) {
      if (
        !(error instanceof UnresolvedSurface) ||
        (!options.bestEffort && !(error instanceof OutsideExportFrame))
      )
        throw error;
      warnings.push(`Receiver ${receiver.id}: navigation binding omitted; ${error.message}`);
      return [];
    }
    if (receiver.shape.projection_area)
      throw new Error(`${receiver.id}: projection volume already has a receiving area`);
    receiver.shape.projection_area = [area.sector, area.layer];
    return { receiver, area };
  });
  // Receiving heights also resolve doors, masks and other local feature anchors.
  // These are lookup aliases for the same sector, never extra movement polygons.
  for (const { receiver, area } of boundReceivers) {
    const top = receiver.shape.points.map((p): Vec3 => [p.x, p.y - p.z_top, p.z_top]);
    const plane = heightPlane(top.slice(0, 3));
    const coverage = fixedPolygonBoolean(
      "intersection",
      [area.polygon],
      [[top.map(([x, y]): Point => [x, y])]],
    );
    for (const region of coverage)
      areas.push({
        ...area,
        plane,
        polygon: region[0]!,
        blockers: [...area.blockers, ...region.slice(1)],
      });
  }
  options.onProgress?.("Constructing masks");
  const masks: NonNullable<CompiledAssetGeometry["masks"]> = [];
  const maskIndices = new Map<string, number[]>();
  for (const mask of placedMasks) {
    if (mask.rules.mask_type === 0) {
      maskIndices.set(mask.id, []);
      continue;
    }
    if (cropped && outsideAnchor(mask.anchor)) {
      warnings.push(`Mask ${mask.id}: omitted because its anchor lies outside the export frame.`);
      maskIndices.set(mask.id, []);
      continue;
    }
    // Masks select a receiving layer, not a movement destination. A placed
    // obstacle may cover their anchor without removing the authored receiver.
    const point = project(mask.anchor);
    const heightPoint: Point = [mask.anchor[0], mask.anchor[1] - mask.anchor[2]];
    let segmentError: string | undefined;
    const probe =
      mask.receiverPolylines ??
      (mask.receiverPolyline
        ? [mask.receiverPolyline]
        : mask.receiverSegment
          ? [mask.receiverSegment]
          : undefined);
    const receivingPoints = (plane: HeightPlane): Point[] => {
      if (mask.receiverPoints)
        return mask.receiverPoints
          .filter((p) => Math.abs(planeHeight(plane, [p[0], p[1] - p[2]]) - p[2]) < 1e-4)
          .map(project);
      if (!probe)
        return Math.abs(planeHeight(plane, heightPoint) - mask.anchor[2]) < 1e-4 ? [point] : [];
      try {
        return probe.flatMap((line) =>
          line.slice(1).flatMap((end, i): Point[] => {
            const intersection = lightReceiverIntersection(
              [line[i]!, end],
              plane,
              `Mask ${mask.id}`,
            );
            return intersection ? [[intersection[0], intersection[1] - intersection[2]]] : [];
          }),
        );
      } catch (error) {
        if (!options.bestEffort || !(error instanceof Error)) throw error;
        segmentError = error.message;
        return [];
      }
    };
    const receivers = groups.filter((group) => {
      return receivingPoints(group.plane).some((p) =>
        group.surfaces.some(
          (surface) => inside(p, surface.polygon) && !surface.holes.some((hole) => inside(p, hole)),
        ),
      );
    });
    let receivingLayers = new Set(
      areas
        .filter((area) => {
          return receivingPoints(area.plane).some((p) => inside(p, area.polygon));
        })
        .map((area) => area.layer),
    );
    if (!receivingLayers.size || probe || mask.receiverPoints)
      receivingLayers = new Set([
        ...receivingLayers,
        ...receivers.flatMap((group) =>
          areas
            .filter(
              (area) =>
                area.lift === group.lift &&
                area.navigationRegion === group.navigationRegion &&
                (group.navigationRegion !== undefined ||
                  area.plane.every((n, i) => Math.abs(n - group.plane[i]!) < 1e-7)),
            )
            .map((area) => area.layer),
        ),
      ]);
    if (segmentError || receivingLayers.size !== 1) {
      if (options.bestEffort) {
        warnings.push(
          `Mask ${mask.id}: omitted because ${segmentError ?? `its receiving layer is unavailable or ambiguous (found ${receivingLayers.size})`}; sprite occlusion is incomplete.`,
        );
        maskIndices.set(mask.id, []);
        continue;
      }
      throw new Error(
        `${mask.id} receiving anchor must resolve to exactly one authored receiving layer (found ${receivingLayers.size})`,
      );
    }
    const layer = [...receivingLayers][0]!;
    const tiles = rasterizeMaskGeometry(
      mask.triangles,
      { ...mask.rules, layer },
      mask.cullBackfaces,
      mask.alphaCoverage,
    );
    maskIndices.set(
      mask.id,
      tiles.map((_, index) => masks.length + index),
    );
    masks.push(...tiles);
  }
  if (masks.length > 65536) throw new Error("Too many compiled mask tiles");
  const maskRefs = (ids: string[]) =>
    ids.flatMap((id) => {
      const indices = maskIndices.get(id);
      if (!indices) throw new Error(`Unresolved transition mask ${id}`);
      return indices;
    });
  // Airborne jumps do not collision-check each frame. Any volume that can
  // become active must still constrain the permanently generated jump span.
  options.onProgress?.("Connecting jumps and doors");
  const changingSight = new Set(
    transitions.flatMap((transition) => [...transition.initialSight, ...transition.appliedSight]),
  );
  const jumpReceivers = new Map<string, JumpReceivingSurface>(generatedLandings);
  const flightClearance =
    jumpPairs.length || jumpSegments.length
      ? createJumpClearance(
          sight.map((shape, index) =>
            changingSight.has(index) && shape.initial_active === false
              ? { ...shape, initial_active: true }
              : shape,
          ),
          jumpReceivers,
        )
      : undefined;
  const walkingClearance = createJumpWalkingClearance(jumpWalkAreas, generatedLandings);
  const assembledJumps = assembleJumpSegments(
    jumpSegments,
    (edges, long, body) => {
      for (const edge of edges) {
        const band = generatedLandings.get(edge.zone);
        const zone = band
          ? jumpLandingBand(edge.zone, edge, band)
          : jumpZones.find((zone) => zone.id === edge.zone);
        if (!zone) throw new Error(`Jump ${edge.zone}: missing receiving zone`);
        const area = resolve(zone.anchor, `${edge.zone} takeoff receiver`);
        jumpReceivers.set(edge.zone, {
          ...jumpReceivers.get(edge.zone),
          topology: { sector: area.sector, layer: area.layer },
          motionPolygon: area.polygon,
        });
      }
      return mergeIntervals([...flightClearance!(edges, long, body), ...walkingClearance(edges)]);
    },
    generatedLandings.size ? "obstacles obstruct the flight or walking approach" : undefined,
  );
  for (const pair of assembledJumps.pairs) {
    for (const [side, edge] of pair.edges.entries()) {
      const band = generatedLandings.get(edge.zone);
      if (!band) continue;
      const id = `${pair.id}/landing-${side}`;
      const zone = jumpLandingBand(id, edge, band);
      zone.polygon = ring(
        zone.polygon.map(([x, y]): Point => [quantize(x), quantize(y)]),
        id,
      );
      jumpZones.push(zone);
      generatedJumpZoneIds.add(id);
      edge.zone = id;
    }
    jumpPairs.push(pair);
  }
  warnings.push(...assembledJumps.warnings);
  for (const segment of assembledJumps.unmatched.filter(
    (segment) => !segment.long || !generatedLandings.has(segment.edge.zone),
  ))
    warnings.push(
      `Jump ${segment.id}: no matching edge after placement; connection is unavailable.`,
    );
  // Detached edges have no runtime connection. Retain zones used by any remaining pair.
  if (options.bestEffort || cropped || generatedJumpZoneIds.size) {
    const unavailable = new Set<string>();
    for (const zone of jumpZones) {
      if (!options.bestEffort && !cropped && !generatedJumpZoneIds.has(zone.id)) continue;
      try {
        resolve(zone.anchor, `${zone.id} landing anchor`);
      } catch (error) {
        if (
          !(error instanceof UnresolvedSurface) ||
          (!options.bestEffort &&
            !generatedJumpZoneIds.has(zone.id) &&
            !(error instanceof OutsideExportFrame))
        )
          throw error;
        unavailable.add(zone.id);
      }
    }
    for (let index = jumpPairs.length - 1; index >= 0; index--) {
      const pair = jumpPairs[index]!;
      if (!pair.edges.some((edge) => unavailable.has(edge.zone))) continue;
      warnings.push(
        `Jump ${pair.id}: connection omitted because a landing surface is unavailable.`,
      );
      jumpPairs.splice(index, 1);
    }
  }
  const usedJumpZones = new Set(jumpPairs.flatMap((pair) => pair.edges.map((edge) => edge.zone)));
  const activeJumpZones = jumpZones.filter((zone) => usedJumpZones.has(zone.id));
  const compiledJumpZones = activeJumpZones.map((zone) => {
    const area = resolve(zone.anchor, `${zone.id} landing anchor`);
    jumpReceivers.set(zone.id, {
      ...jumpReceivers.get(zone.id),
      topology: { sector: area.sector, layer: area.layer },
      motionPolygon: area.polygon,
    });
    return {
      polygon: { points: zone.polygon },
      sector: area.sector,
      layer: area.layer,
      helper_needed: zone.helper,
    };
  });
  const compiledJumpPairs = jumpPairs.map((pair) => {
    const indices = pair.edges.map((edge) =>
      activeJumpZones.findIndex((zone) => zone.id === edge.zone),
    );
    const lines = pair.edges.map((edge, i) => {
      // Edge heights are authored independently of the receiving surface's plane.
      // In particular, integer edge heights need not equal fractional projection heights.
      // Match generated ledges: integer flight endpoints must not round down
      // into the supporting surface. Keep the projected contact coordinates.
      const a: Vec3 = [...project(edge.a), quantize(Math.ceil(edge.a[2] - 1e-4) + 0)];
      const b: Vec3 = [...project(edge.b), quantize(Math.ceil(edge.b[2] - 1e-4) + 0)];
      if (a[0] === b[0] && a[1] === b[1])
        throw new Error(`${pair.id}: jump edge collapses on the movement grid`);
      return { point_a: a, point_b: b, jump_zone_index: indices[1 - i]! };
    });
    const [first, second] = pair.edges;
    const vector = (edge: (typeof pair.edges)[number]): Point => [
      edge.b[0] - edge.a[0],
      edge.b[1] - edge.b[2] - edge.a[1] + edge.a[2],
    ];
    const u = vector(first!),
      v = vector(second!);
    if (u.every((value, axis) => Math.abs(value + v[axis]!) < 1e-7)) {
      // Runtime landing translation uses the source edge's vector in either
      // direction. Round paired parallel spans together, preserving heights.
      for (const axis of [0, 1])
        lines[1]!.point_a[axis] = quantize(
          lines[1]!.point_b[axis]! + lines[0]!.point_b[axis]! - lines[0]!.point_a[axis]!,
        );
    }
    return { line1: lines[0]!, line2: lines[1]!, jump_long: pair.long };
  });
  for (const [index, pair] of compiledJumpPairs.entries())
    for (const issue of auditCompiledJump(pair, flightClearance!, [
      jumpPairs[index]!.edges[0]!.zone,
      jumpPairs[index]!.edges[1]!.zone,
    ]))
      warnings.push(`Jump ${jumpPairs[index]!.id}: ${issue}.`);
  // Runtime construction order is motion, materials, projection planes, then buildings.
  // Motion adds an out-of-map sector; each door also consumes a constructor slot.
  // Physical endpoints reach the runtime unchanged. Rounding before binding
  // can move a supported approach outside a narrow receiving roof.
  const doorProjection = (door: (typeof doors)[number], point: Vec3): Point =>
    door.lift && physicalStairs.has(door.lift) ? [point[0], point[1] - point[2]] : project(point);
  const resolveDoorOutside = (door: (typeof doors)[number], lift?: string | null) => {
    const label = `${door.name} outside`;
    if (door.outsideReceiverSegment) {
      // Move the runtime approach point onto the receiver as well as selecting its sector.
      door.outside = resolveReceivingSegment(door.outsideReceiverSegment, door.outside, label);
      door.outsideAnchor = door.outside;
    }
    return resolve(
      door.outsideAnchor,
      label,
      lift,
      false,
      doorProjection(door, door.outsideAnchor),
    );
  };
  const resolveDoorInside = (door: (typeof doors)[number], lift?: string | null) => {
    const label = `${door.name} inside`;
    if (door.insideReceiverSegment) {
      door.inside = resolveReceivingSegment(door.insideReceiverSegment, door.inside, label);
      door.insideAnchor = door.inside;
    }
    return resolve(door.insideAnchor, label, lift, false, doorProjection(door, door.insideAnchor));
  };
  const omittedDoors = new Set<string>();
  if (options.bestEffort || cropped) {
    for (const door of doors.filter((door) => door.lift)) {
      try {
        resolveDoorOutside(door);
        resolveDoorInside(door, door.lift);
      } catch (error) {
        if (!(error instanceof UnresolvedSurface)) throw error;
        const owner = placements
          .filter((placement) => door.name.startsWith(`${placement.id}/`))
          .sort((a, b) => b.id.length - a.id.length)[0];
        if (!owner) throw error;
        if (!options.bestEffort && !(error instanceof OutsideExportFrame)) throw error;
        throw new UnavailableLiftPlacement(
          door.lift!,
          error.message,
          error instanceof OutsideExportFrame,
          [...assembledLifts.identities].filter(([, id]) => id === door.lift).map(([id]) => id),
        );
      }
    }
    for (let index = doors.length - 1; index >= 0; index--) {
      const door = doors[index]!;
      // Lift door counts and reserved areas form one assembly and remain strict.
      if (door.lift) continue;
      let reason: string | undefined;
      try {
        const outside = resolveDoorOutside(door, null);
        const inside = door.interior ? undefined : resolveDoorInside(door, null);
        if (options.bestEffort && inside && outside.sector === inside.sector)
          reason = "both endpoints share a movement area";
      } catch (error) {
        if (
          !(error instanceof UnresolvedSurface) ||
          (!options.bestEffort && !(error instanceof OutsideExportFrame))
        )
          throw error;
        reason = error.message;
      }
      if (!reason) continue;
      warnings.push(`Door ${door.name}: omitted because ${reason}.`);
      omittedDoors.add(door.name);
      doors.splice(index, 1);
    }
    for (const transition of transitions) {
      if (!transition.doorLinks) continue;
      transition.doorLinks.ids = transition.doorLinks.ids.filter((id) => !omittedDoors.has(id));
      if (!transition.doorLinks.ids.length) delete transition.doorLinks;
    }
    interiors = interiors.filter((id) => doors.some((door) => door.interior === id));
  }
  let nextInteriorSector =
    sector + 1 + materials.length + sight.filter((o) => o.projection_area !== null).length;
  const interiorAreas = new Map(
    interiors.map((id) => {
      const area = { sector: nextInteriorSector, layer: layers.length - 1 };
      nextInteriorSector += 1 + doors.filter((d) => d.interior === id).length;
      return [id, area] as const;
    }),
  );
  const compiledDoors = doors.map((door) => {
    // Ordinary passages can meet traversal surfaces; lift doors retain their explicit owner.
    const outside = resolveDoorOutside(door, door.lift ? undefined : null),
      inside = door.interior
        ? interiorAreas.get(door.interior)!
        : resolveDoorInside(door, door.lift ?? null);
    if (outside.sector === inside.sector) {
      if (
        !door.definition.allowContinuous ||
        transitions.some((t) => t.doorLinks?.ids.includes(door.name))
      )
        throw new Error(`${door.name} does not connect distinct motion areas`);
      omittedDoors.add(door.name);
      warnings.push(
        `${door.name}: omitted unrestricted passage because both endpoints now share a movement area`,
      );
      return null;
    }
    const d = door.definition;
    return {
      ...(!door.lift && !door.interior
        ? {
            world_endpoints: {
              inside: door.inside,
              middle: door.worldMiddle,
              outside: door.outside,
            },
          }
        : {}),
      door_type: d.type,
      active: d.active ?? true,
      locked_pc: d.locked,
      unlockable: d.unlockable,
      locked_npc_villain: d.lockedVillains ?? false,
      locked_npc_civilian: d.lockedCivilians ?? false,
      locked_pc_after_patch: d.afterTransition?.locked ?? d.locked,
      unlockable_after_patch: d.afterTransition?.unlockable ?? d.unlockable,
      locked_npc_villain_after_patch:
        d.afterTransition?.lockedVillains ?? d.lockedVillains ?? false,
      locked_npc_civilian_after_patch:
        d.afterTransition?.lockedCivilians ?? d.lockedCivilians ?? false,
      door_sector: { points: door.polygon },
      point_out: project(door.outside),
      sector_out: outside.sector,
      layer_out: outside.layer,
      point_mid: door.middle,
      point_in: project(door.inside),
      sector_in: inside.sector,
      layer_in: inside.layer,
    };
  });
  const selectDoors = (predicate: (door: (typeof doors)[number]) => boolean) =>
    compiledDoors.flatMap((compiled, i) => (compiled && predicate(doors[i]!) ? [compiled] : []));
  // Native non-lift door allocation follows interiors, then standalone passages.
  const patchDoors = [
    ...interiors.flatMap((id) => doors.filter((door) => door.interior === id)),
    ...doors.filter((door) => !door.lift && !door.interior),
  ].filter((door) => !omittedDoors.has(door.name));
  const doorIndices = new Map(patchDoors.map((door, index) => [door.name, index]));
  options.onProgress?.("Constructing lighting and state controls");
  const lightCoverage = new Map<string, Point[][]>();
  for (const light of lights) {
    if (!light.receiverGroup) continue;
    const contours = lightCoverage.get(light.receiverGroup) ?? [];
    contours.push(light.receiverContour, light.polygon);
    lightCoverage.set(light.receiverGroup, contours);
  }
  options.onProgress?.("Compiling navigation graph");
  let graphBytes: number[] = [];
  try {
    graphBytes = compileNavigationGraph(layers);
  } catch (error) {
    if (!options.bestEffort) throw error;
    warnings.push(
      `Navigation graph omitted: ${error instanceof Error ? error.message : String(error)}. Indirect movement routes are unavailable.`,
    );
  }
  const compiled: CompiledAssetGeometry = {
    ...(warnings.length ? { warnings } : {}),
    motion_data: { layers, graph_bytes: graphBytes },
    ...(masks.length ? { masks } : {}),
    ...(compiledJumpZones.length
      ? { jump_zones: compiledJumpZones, jump_line_pairs: compiledJumpPairs }
      : {}),
    ...(lights.length
      ? {
          light_sectors: lights.flatMap((light) => {
            if (light.receivers || light.receiverSegments || light.receiverPolylines) {
              const probes = [
                ...(light.receiverSegments ?? []),
                ...(light.receiverPolylines ?? []),
              ];
              const segmentReceivers = probes.flatMap((probe, index) => {
                const matches = areas.flatMap((area) => {
                  return probe.slice(1).flatMap((end, i) => {
                    const point = lightReceiverIntersection([probe[i]!, end], area.plane);
                    return point && inside([point[0], point[1] - point[2]], area.polygon)
                      ? [{ area, point }]
                      : [];
                  });
                });
                if (new Set(matches.map(({ area }) => area.sector)).size !== 1) {
                  const candidates = matches.map(({ area, point }) => ({
                    sector: area.sector,
                    layer: area.layer,
                    height: point[2],
                  }));
                  const message = `${light.id}: receiving segment ${index} must intersect exactly one walkable surface; intersections ${JSON.stringify(candidates)}`;
                  if (!options.bestEffort) throw new Error(message);
                  warnings.push(`Light receiver omitted: ${message}.`);
                  return [];
                }
                return [matches[0]!.point];
              });
              const layers = new Set(
                [...(light.receivers ?? []), ...segmentReceivers].flatMap((point, index) => {
                  // Layer-selection anchors can belong to the authored contour
                  // or its emitted integer contour. Rounding either side alone
                  // can reject valid probes, including deformed spline probes.
                  const projected: Point = [point[0], point[1] - point[2]];
                  const coverage = light.receiverGroup
                    ? lightCoverage.get(light.receiverGroup)!
                    : [light.receiverContour, light.polygon];
                  if (!coverage.some((contour) => inside(projected, contour)))
                    throw new Error(
                      `${light.id}: receiver ${index} lies outside the light contour`,
                      { cause: { receiver: point, projected, coverage } },
                    );
                  try {
                    return [
                      resolve(point, `${light.id} receiver ${index}`, null, true, projected).layer,
                    ];
                  } catch (error) {
                    if (
                      !(error instanceof UnresolvedSurface) ||
                      (!options.bestEffort && !(error instanceof OutsideExportFrame))
                    )
                      throw error;
                    warnings.push(`Light receiver omitted: ${error.message}`);
                    return [];
                  }
                }),
              );
              return [...layers].map((layer) => ({
                layer,
                polygon: { points: light.polygon },
                ambience: light.ambiences,
              }));
            }
            const matchingLayers = new Set(
              areas
                .filter(
                  (area) =>
                    area.plane.every((n, i) => Math.abs(n - light.plane[i]!) < 1e-7) &&
                    polygonClipping.intersection([area.polygon], [light.polygon]).length > 0,
                )
                .map((area) => area.layer),
            );
            if (matchingLayers.size === 0) {
              if (
                options.bestEffort ||
                !fixedPolygonBoolean("intersection", polygon(light.polygon), [frame]).length
              ) {
                warnings.push(
                  `Light region ${light.id}: omitted because no receiving layer overlaps its contour.`,
                );
                return [];
              }
              throw new Error(
                `${light.id}: light region must overlap at least one receiving layer (found ${matchingLayers.size})`,
              );
            }
            return [...matchingLayers].map((layer) => ({
              layer,
              polygon: { points: light.polygon },
              ambience: light.ambiences,
            }));
          }),
        }
      : {}),
    ...(transitions.length
      ? {
          movement_transitions: transitions.flatMap((t) => {
            const initialMasks = maskRefs(t.initialMasks);
            const appliedMasks = maskRefs(t.appliedMasks);
            if (
              !t.changes.length &&
              !t.initialSight.length &&
              !t.appliedSight.length &&
              !initialMasks.length &&
              !appliedMasks.length &&
              !t.hasAppearance &&
              !t.doorLinks
            ) {
              if (options.bestEffort) {
                warnings.push(
                  `Transition ${t.id}: omitted because none of its gameplay effects are available.`,
                );
                return [];
              }
              throw new Error(`${t.id}: movement transition affects no walkable area`);
            }
            // State reference points identify a surface even inside its collision contours.
            // Door receiving anchors and jump landing anchors require an unblocked position.
            let area;
            try {
              const anchor = t.waypointReceiverSegment
                ? resolveReceivingSegment(
                    t.waypointReceiverSegment,
                    t.waypointAnchor,
                    `${t.id} waypoint`,
                    true,
                  )
                : t.waypointAnchor;
              area = resolve(anchor, `${t.id} waypoint`, undefined, true);
              if (t.waypointReceiverSegment) {
                const before = project(t.waypoint),
                  after = project(anchor);
                const shift = ([x, y]: Point): Point => [
                  x + after[0] - before[0],
                  y + after[1] - before[1],
                ];
                t.applyPolygon = t.applyPolygon.map(shift);
                t.noApplyPolygon = t.noApplyPolygon.map(shift);
                t.waypoint = anchor;
              }
            } catch (error) {
              if (
                !(error instanceof UnresolvedSurface) ||
                (!options.bestEffort && !(error instanceof OutsideExportFrame))
              )
                throw error;
              if (
                t.changes.length ||
                t.initialMasks.length ||
                t.appliedMasks.length ||
                t.initialSight.length ||
                t.appliedSight.length
              )
                throw new UnavailableStateControl(
                  t.id,
                  error.message,
                  error instanceof OutsideExportFrame,
                );
              warnings.push(`Transition ${t.id}: omitted; ${error.message}`);
              return [];
            }
            return {
              id: t.id,
              ...(t.hasAppearance ? { has_appearance: true } : {}),
              waypoint: project(t.waypoint),
              sector: area.sector,
              layer: area.layer,
              active: t.active,
              definitive: t.definitive,
              apply_polygon: { points: t.applyPolygon },
              no_apply_polygon: { points: t.noApplyPolygon },
              motion_changes: t.changes,
              ...(t.initialSight.length ? { initial_sight: t.initialSight } : {}),
              ...(t.appliedSight.length ? { applied_sight: t.appliedSight } : {}),
              ...(initialMasks.length ? { initial_masks: initialMasks } : {}),
              ...(appliedMasks.length ? { applied_masks: appliedMasks } : {}),
              ...(t.doorLinks
                ? {
                    door_links: {
                      mode: t.doorLinks.mode,
                      indices: t.doorLinks.ids.map((id) => {
                        const index = doorIndices.get(id);
                        if (index === undefined || index > 65535)
                          throw new Error(`${t.id}: unresolved transition door ${id}`);
                        return index;
                      }),
                    },
                  }
                : {}),
            };
          }),
        }
      : {}),
    ...(mapSettings ? { map_settings: mapSettings } : {}),
    ...(sounds.length ? { sound_sources: sounds } : {}),
    ...(animations.length ? { animations } : {}),
    sight_obstacles: sight,
    ...(materials.length
      ? { material_sectors: materials, sight_material_indices: groundMaterials }
      : {}),
    doors: selectDoors((door) => !door.lift && !door.interior),
    ...(interiors.length
      ? {
          buildings: interiors.map((id) => ({
            Building: {
              doors: selectDoors((door) => door.interior === id),
            },
          })),
        }
      : {}),
    ...(lifts.length
      ? {
          lifts: lifts.map((lift) => {
            const area = areas.find((a) => a.lift === lift.id);
            if (!area) throw new Error(`Missing lift motion area ${lift.id}`);
            const endpoints = selectDoors((door) => door.lift === lift.id);
            const placedEndpoints = doors.filter(
              (door, index) => door.lift === lift.id && compiledDoors[index] !== null,
            );
            const physical = physicalStairs.get(lift.id)?.navigation;
            const physicalDoorIndices = doors.flatMap((door, index) =>
              door.lift === lift.id ? [index] : [],
            );
            // Heights and stable local door order survive arbitrary placement;
            // projected screen Y does not identify the top of a rotated lift.
            const ranked = placedEndpoints
              .map((door, index) => ({ index, height: door.outside[2] }))
              .sort((a, b) => a.height - b.height || a.index - b.index);
            const invalid =
              endpoints.length < 2 || !endpoints.some((d) => d.door_type === 5)
                ? `Lift ${lift.id} needs at least two traversal doors including a low door`
                : undefined;
            if (invalid) {
              if (!options.bestEffort) throw new Error(invalid);
              throw new UnavailableLiftPlacement(
                lift.id,
                invalid,
                false,
                [...assembledLifts.identities].filter(([, id]) => id === lift.id).map(([id]) => id),
              );
            }
            return {
              motion_area_index: area.sector,
              lift_type: lift.type,
              direction: lift.direction,
              endpoint_doors: [ranked[0]!.index, ranked.at(-1)!.index] as [number, number],
              doors: endpoints,
              ...(physical
                ? {
                    physical_navigation: {
                      ...physical,
                      doors: physical.doors.filter(
                        (_, index) => compiledDoors[physicalDoorIndices[index]!] !== null,
                      ),
                    },
                  }
                : {}),
            };
          }),
        }
      : {}),
  };
  options.onProgress?.("Finalizing sight and appearance bindings");
  const assembled = assembleSightVolumes(compiled, sightJoins, sightCaps);
  orderSightVolumes(compiled, sightOrders, assembled);
  if (compiled.movement_transitions)
    compiled.movement_transitions = assembleTransitions(
      compiled.movement_transitions,
      transitionJoins,
    );
  compileAppearanceBindings(
    document,
    descriptors,
    compiled.movement_transitions,
    options.bestEffort ? (message) => warnings.push(message) : undefined,
  );
  if (warnings.length) compiled.warnings = warnings;
  return compiled;
}
