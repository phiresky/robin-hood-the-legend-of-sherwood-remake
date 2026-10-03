import { terrainGameplay } from "./authored-terrain.ts";
import { wallSplineGameplay } from "./wall-spline-gameplay.ts";
import polygonClipping, { type Polygon } from "polygon-clipping";
import { assembleSightVolumes } from "./assemble-sight-volumes.ts";
import { orderSightVolumes } from "./order-sight-volumes.ts";
import { compileSoundSource } from "./compile-sound-source.ts";
import { fixedPolygonBoolean } from "./fixed-polygon-boolean.ts";
import { unionMovementSurfaces } from "./union-movement-surfaces.ts";
import { assembleNavigationRegions, type NavigationPiece } from "./assemble-navigation-regions.ts";
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
import { createJumpClearance } from "./jump-clearance.ts";
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

import { heightPlane, planeHeight, clipHeight, type HeightPlane } from "./gameplay-plane.ts";
import { quantizeGeneratedMotionPolygon, simplifyMotionRing } from "./motion-quantization.ts";
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
function inside(p: Point, points: Point[]) {
  let hit = false;
  for (let i = 0, j = points.length - 1; i < points.length; j = i++) {
    const a = points[i]!,
      b = points[j]!;
    if (
      a[1] > p[1] !== b[1] > p[1] &&
      p[0] < ((b[0] - a[0]) * (p[1] - a[1])) / (b[1] - a[1]) + a[0]
    )
      hit = !hit;
  }
  return hit;
}
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

/** Compile only placed editor assets. There is deliberately no datadir, source-map or level-record input. */
export function compileAssetGameplay(
  document: Level3D,
  descriptors: ReadonlyMap<string, ProjectionAssetDescriptor>,
  bounds: [number, number, number, number],
  options: { bestEffort?: boolean } = {},
): CompiledAssetGeometry {
  const omitted = new Set<string>();
  const fixedTransitions = new Set<string>();
  const omissions: string[] = [];
  for (;;) {
    try {
      const result = compileAssetGameplayAttempt(
        document,
        descriptors,
        bounds,
        options,
        omitted,
        fixedTransitions,
      );
      if (omissions.length) result.warnings = [...omissions, ...(result.warnings ?? [])];
      return result;
    } catch (error) {
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
  options: { bestEffort?: boolean },
  omitted: ReadonlySet<string>,
  fixedTransitions: ReadonlySet<string>,
): CompiledAssetGeometry {
  const warnings: string[] = [];
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
  const terrain = terrainGameplay(document);
  const walls = wallSplineGameplay(document, descriptors, !!options.bestEffort);
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
    lift?: string;
    navigationRegion?: string;
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
  const movementSolids: { owner: string; shape: SightObstacle }[] = [];
  const movementClearances: typeof surfaces = [];
  const transitionBlockers: PlacedTransitionBlocker[] = [];
  const projectionSupports: (ProjectionMaterialSupport & {
    plane: HeightPlane;
    holes: Point[][];
    navigationRegion?: string;
    lift?: string;
  })[] = [];
  const lights: {
    id: string;
    polygon: Point[];
    plane: HeightPlane;
    ambiences: number;
    receivers?: Vec3[];
    receiverSegments?: [Vec3, Vec3][];
  }[] = [];
  const placedMasks: {
    id: string;
    anchor: Vec3;
    receiverSegment?: [Vec3, Vec3];
    triangles: MaskTriangle[];
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
    polygon: Point[];
  }[] = [];
  const sight: SightObstacle[] = [];
  const sightJoins: import("./assemble-sight-volumes.ts").PlacedSightJoin[] = [];
  const sightOrders = new Map<number, number>();
  const sightCaps: import("./assemble-sight-volumes.ts").PlacedSightCap[] = [];
  const materials: NonNullable<CompiledAssetGeometry["material_sectors"]> = [];
  const groundMaterials: number[] = [];
  const sounds: NonNullable<CompiledAssetGeometry["sound_sources"]> = [];
  let mapSettings: CompiledAssetGeometry["map_settings"];
  const quantize = (n: number) => {
    const result = Math.round(n);
    if (!Number.isFinite(n) || result < -32768 || result > 32767)
      throw new Error("Asset gameplay exceeds signed 16-bit coordinates");
    return result;
  };
  for (const placement of placements) {
    const authored = placement.descriptor.gameplay!;
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
        polygon: ring(points.map(project), `${placement.id}/${light.id}`),
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
      });
    }
    for (const sound of gameplay.sounds ?? []) sounds.push(compileSoundSource(sound, transform));
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
      if (movementSolid(volume.id)) movementSolids.push({ owner: placement.id, shape });
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
      projectionReceivers.push({
        id: `${placement.id}/${receiver.id}`,
        anchor: transform(receiver.node, receiver.anchor),
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
      const boundary = (points: Vec3[] | undefined, projected: boolean, closed = true) =>
        points
          ? maskBoundaryPolyline(
              points.map((point): Point => {
                const p = transform(mask.node, point);
                return projected ? project(p) : [quantize(p[0]), quantize(p[1])];
              }),
              closed,
            )
          : null;
      placedMasks.push({
        id: `${placement.id}/${mask.id}`,
        anchor: transform(mask.node, mask.anchor),
        ...(mask.receiverSegment
          ? {
              receiverSegment: [
                transform(mask.node, mask.receiverSegment[0]),
                transform(mask.node, mask.receiverSegment[1]),
              ] as [Vec3, Vec3],
            }
          : {}),
        triangles: mask.triangles.map(([a, b, c]) => [
          transform(mask.node, a),
          transform(mask.node, b),
          transform(mask.node, c),
        ]),
        rules: {
          mask_type:
            (mask.characterBoundary ? 1 : 0) |
            (mask.projectileBoundary || mask.obstacles.length ? 2 : 0) |
            (mask.view ? 4 : 0) |
            (mask.obstacles.length ? 16 : 0),
          character_polyline: boundary(mask.characterBoundary, true, mask.characterBoundaryClosed),
          projectile_polyline:
            boundary(mask.projectileBoundary, false, mask.projectileBoundaryClosed) ??
            (mask.obstacles.length ? [] : null),
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
      const local = surface.polygon.map((p, i): Vec3 => [
        p[0],
        p[1],
        typeof surface.height === "number" ? surface.height : surface.height[i]!,
      ]);
      const localPlane = heightPlane(local);
      const anchors = surface.projectionMaterials?.planePoints;
      if (anchors) {
        heightPlane(anchors);
        if (anchors.some(([x, y, z]) => Math.abs(planeHeight(localPlane, [x, y]) - z) > 1e-4))
          throw new Error(`${surface.id}: receiving plane anchors must lie on the surface`);
      }
      const points = local.map((p) => transform(surface.node, p));
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
      const plane = heightPlane(points.map(([x, y, z]) => [x, y - z, z]));
      const target = gameplay.movementClearances?.includes(surface)
        ? movementClearances
        : gameplay.movementBlockers?.includes(surface)
          ? movementBlockers
          : surfaces;
      // Clearances are intermediate cutouts. Snapping their intersections before
      // clipping solids bends otherwise straight movement boundaries.
      const clearance = target === movementClearances;
      const continuous = clearance || surface.preserveMovementPrecision === true;
      const projectMovement = continuous ? ([x, y, z]: Vec3): Point => [x, y - z] : project;
      const minimumArea = continuous ? 1e-8 : 0.5;
      const placed = {
        owner: placement.id,
        preserveMovementBoundary: surface.preserveMovementBoundary,
        holeContours: surface.holeContours,
        movementContour: surface.movementContour,
        navigationRegion:
          surface.navigationRegion === undefined
            ? undefined
            : `${placement.id}/${surface.navigationRegion}`,
        polygon: ring(points.map(projectMovement), `${placement.id}/${surface.id}`, minimumArea),
        plane,
        ...(gameplay.lifts?.find((l) => l.surface === surface.id)
          ? { lift: `${placement.id}/${gameplay.lifts.find((l) => l.surface === surface.id)!.id}` }
          : {}),
        holes: (surface.holes ?? []).map((hole) =>
          ring(
            hole.map((p) =>
              projectMovement(transform(surface.node, [p[0], p[1], planeHeight(localPlane, p)])),
            ),
            `${placement.id}/${surface.id} hole`,
            minimumArea,
          ),
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
                  holes: (surface.holes ?? []).map((hole) =>
                    hole.map((p): Point => {
                      const [x, y] = transform(surface.node, [...p, planeHeight(localPlane, p)]);
                      return [x, y];
                    }),
                  ),
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
  if (terrain) {
    const placed = surfaces.filter((s) => s.owner !== "authored-terrain");
    const ground = surfaces.filter((s) => s.owner === "authored-terrain");
    const replacement: typeof surfaces = [];
    for (const surface of ground) {
      const cuts = placed
        .filter((other) => other.plane.every((n, i) => Math.abs(n - surface.plane[i]!) < 1e-7))
        .map((other) => [other.polygon, ...other.holes] as Polygon);
      const shape: Polygon = [surface.polygon, ...surface.holes];
      const remaining = cuts.length ? polygonClipping.difference(shape, ...cuts) : [shape];
      for (const polygon of remaining)
        replacement.push({
          ...surface,
          polygon: ring(polygon[0]!),
          holes: polygon.slice(1).map((h) => ring(h)),
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
      const height = (a[2] + b[2]) / 2;
      const groundSurface = replacement.find(
        (s) =>
          Math.abs(planeHeight(s.plane, probe) - height) < 1e-4 &&
          inside(probe, s.polygon) &&
          !s.holes.some((h) => inside(probe, h)) &&
          !movementBlockers.some(
            (blocker) =>
              blocker.owner === "authored-terrain" &&
              Math.abs(planeHeight(blocker.plane, probe) - height) < 1e-4 &&
              inside(probe, blocker.polygon),
          ),
      );
      if (groundSurface?.navigationRegion)
        navigationJoins.push({
          owner: "authored-terrain",
          region: groundSurface.navigationRegion,
          edge: [b, a],
        });
    }
  }
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
  // Airborne jumps do not collision-check each frame. Any volume that can
  // become active must still constrain the permanently generated jump span.
  const changingSight = new Set(
    transitions.flatMap((transition) => [...transition.initialSight, ...transition.appliedSight]),
  );
  const assembledJumps = assembleJumpSegments(
    jumpSegments,
    jumpSegments.some((segment) => segment.attachment)
      ? createJumpClearance(
          sight.map((shape, index) =>
            changingSight.has(index) && shape.initial_active === false
              ? { ...shape, initial_active: true }
              : shape,
          ),
        )
      : undefined,
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
    (segment) => !generatedLandings.has(segment.edge.zone),
  ))
    warnings.push(
      `Jump ${segment.id}: no matching edge after placement; connection is unavailable.`,
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
  const areas: {
    plane: HeightPlane;
    lift?: string;
    navigationRegion?: string;
    sector: number;
    layer: number;
    polygon: Point[];
    blockers: Point[][];
  }[] = [];
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
  const solidGeometry = movementSolids
    .filter(({ shape }) => shape.solid)
    .map(({ owner, shape }) => {
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
          shape.points.map((p) => [p.x, p.y, p.z_bottom]),
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
    const worldPlane = heightPlane(
      group[0]!.polygon.map(([x, y]) => {
        const z = planeHeight(plane, [x, y]);
        return [x, y + z, z];
      }),
    );
    const worldBounds = boundsOf(
      group.flatMap((surface) =>
        surface.polygon.map(([x, y]): Point => [x, y + planeHeight(plane, [x, y])]),
      ),
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
      let slice: Point[] = [
        [solidBounds[0], solidBounds[1]],
        [solidBounds[2], solidBounds[1]],
        [solidBounds[2], solidBounds[3]],
        [solidBounds[0], solidBounds[3]],
      ];
      const above = top.map((n, i) => n - worldPlane[i]!) as HeightPlane;
      if (footprint.every((p) => planeHeight(above, p) <= 1e-7)) continue;
      slice = clipHeight(slice, above);
      slice = clipHeight(slice, worldPlane.map((n, i) => n - bottom[i]!) as HeightPlane);
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
            !plane.every((n, i) => Math.abs(n - clearance.plane[i]!) < 1e-7)
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
          else merged = polygonClipping.difference(merged, regions);
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
      const blockers = quantized
        .slice(1)
        .map((r) => ring(r, `Merged movement hole on layer ${layer}`));
      navigationPieces.push({ layer, plane, lift, navigationRegion, polygon: boundary, blockers });
    }
  }
  const navigationRegions = assembleNavigationRegions(navigationPieces, warnings);
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
  // Receiving planes can share a navigation region; their provisional layers are not runtime layers.
  layers.length = 0;
  while (layers.length <= liftLayer) layers.push([]);
  for (const region of navigationRegions) {
    const { layer, lift, polygon: boundary, blockers, pieces } = region;
    const plane = pieces[0]!.plane;
    const changing = compileTransitionObstacles(
      boundary,
      blockers,
      plane,
      transitionBlockers,
      warnings,
      pieces.length > 1 ? pieces : undefined,
      pieces[0]!.preserveMovementBoundary === true,
    );
    if (lift && changing.pairs.size)
      throw new Error(
        `Lift ${lift}: changing traversal surfaces require lift state compilation support`,
      );
    for (const [id, pair] of changing.pairs)
      transitions
        .find((t) => t.id === id)!
        .changes.push({
          layer,
          sector,
          changing_obstacle: pair,
        });
    layers[layer]!.push({
      is_lift: !!lift,
      state_id: 0,
      polygon: { points: boundary },
      skeleton_segments: [],
      flags: 0,
      obstacles: [
        ...blockers.map((points) => ({ state_id: 0, polygon: { points } })),
        ...changing.obstacles,
      ],
    });
    // Projection surfaces provide layer-aware elevation and picking.
    for (const piece of pieces) {
      areas.push({ ...piece, sector, layer, blockers: [...piece.blockers, ...changing.initial] });
      const walkableCoverage =
        piece.preserveMovementBoundary && piece.blockers.length
          ? fixedPolygonBoolean(
              "difference",
              [piece.polygon],
              piece.blockers.map((b) => [b]),
            )
          : [[polygon(piece.polygon)[0]!, ...piece.blockers.map((h) => polygon(h)[0]!)]];
      const supports = (planeSupports.get(piece.plane) ?? []).filter(
        (support) =>
          support.lift === piece.lift &&
          support.navigationRegion === piece.navigationRegion &&
          support.plane.every((n, i) => Math.abs(n - piece.plane[i]!) < 1e-7) &&
          // Receiving footprints may extend into blocked space, but ownership
          // comes from walkable coverage, excluding separate islands in holes.
          fixedPolygonBoolean(
            "intersection",
            [polygon(support.polygon)[0]!, ...support.holes.map((h) => polygon(h)[0]!)],
            walkableCoverage,
          ).length > 0,
      );
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
      for (const material of partitionProjectionMaterials(piece.polygon, supports, warnings)) {
        if (material.obstacleIndex !== undefined) continue;
        if (!lift && !material.explicit && !piece.plane.some((n) => Math.abs(n) > 1e-7)) continue;
        const receivingPlane = material.planePoints
          ? heightPlane(material.planePoints.map(([x, y, z]) => [x, y - z, z]))
          : piece.plane;
        sight.push({
          ...(material.planePoints ? { projection_plane: material.planePoints } : {}),
          points: material.polygon.map(([x, y]) => {
            const height = planeHeight(receivingPlane, [x, y]);
            return { x, y: y + height, z_bottom: height, z_top: height };
          }),
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
    sector += 1 + blockers.length + changing.obstacles.length;
  }
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
        Math.abs(planeHeight(a.plane, [point[0], point[1] - point[2]]) - point[2]) < 1e-4 &&
        inside(projected, a.polygon) &&
        (allowBlocked || !a.blockers.some((b) => inside(projected, b))),
    );
    if (new Set(matches.map((a) => a.sector)).size !== 1) {
      const containing = areas.filter((a) => inside(projected, a.polygon));
      const details = containing.slice(0, 8).map((a) => ({
        sector: a.sector,
        layer: a.layer,
        lift: a.lift,
        height: planeHeight(a.plane, [point[0], point[1] - point[2]]),
        blocked: a.blockers.some((b) => inside(projected, b)),
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
  const masks: NonNullable<CompiledAssetGeometry["masks"]> = [];
  const maskIndices = new Map<string, number[]>();
  for (const mask of placedMasks) {
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
    const receivingPoint = (plane: HeightPlane): Point | undefined => {
      if (!mask.receiverSegment)
        return Math.abs(planeHeight(plane, heightPoint) - mask.anchor[2]) < 1e-4
          ? point
          : undefined;
      try {
        const intersection = lightReceiverIntersection(
          mask.receiverSegment,
          plane,
          `Mask ${mask.id}`,
        );
        return intersection ? [intersection[0], intersection[1] - intersection[2]] : undefined;
      } catch (error) {
        if (!options.bestEffort || !(error instanceof Error)) throw error;
        segmentError = error.message;
        return undefined;
      }
    };
    const receivers = groups.filter((group) => {
      const p = receivingPoint(group.plane);
      return (
        p !== undefined &&
        group.surfaces.some(
          (surface) => inside(p, surface.polygon) && !surface.holes.some((hole) => inside(p, hole)),
        )
      );
    });
    let receivingLayers = new Set(
      areas
        .filter((area) => {
          const p = receivingPoint(area.plane);
          return p !== undefined && inside(p, area.polygon);
        })
        .map((area) => area.layer),
    );
    if (!receivingLayers.size || mask.receiverSegment)
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
    const tiles = rasterizeMaskGeometry(mask.triangles, { ...mask.rules, layer });
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
      const a: Vec3 = [...project(edge.a), quantize(edge.a[2])];
      const b: Vec3 = [...project(edge.b), quantize(edge.b[2])];
      if (a[0] === b[0] && a[1] === b[1])
        throw new Error(`${pair.id}: jump edge collapses on the movement grid`);
      return { point_a: a, point_b: b, jump_zone_index: indices[1 - i]! };
    });
    return { line1: lines[0]!, line2: lines[1]!, jump_long: pair.long };
  });
  // Runtime construction order is motion, materials, projection planes, then buildings.
  // Motion adds an out-of-map sector; each door also consumes a constructor slot.
  const resolveDoorOutside = (door: (typeof doors)[number], lift?: string | null) => {
    const label = `${door.name} outside`;
    if (door.outsideReceiverSegment) {
      // Move the runtime approach point onto the receiver as well as selecting its sector.
      door.outside = resolveReceivingSegment(door.outsideReceiverSegment, door.outside, label);
      door.outsideAnchor = door.outside;
    }
    return resolve(door.outsideAnchor, label, lift);
  };
  const resolveDoorInside = (door: (typeof doors)[number], lift?: string | null) => {
    const label = `${door.name} inside`;
    if (door.insideReceiverSegment) {
      door.inside = resolveReceivingSegment(door.insideReceiverSegment, door.inside, label);
      door.insideAnchor = door.inside;
    }
    return resolve(door.insideAnchor, label, lift);
  };
  const omittedDoors = new Set<string>();
  if (options.bestEffort || cropped) {
    for (const door of doors.filter((door) => door.lift)) {
      try {
        resolve(door.outsideAnchor, `${door.name} outside`);
        resolve(door.insideAnchor, `${door.name} inside`, door.lift);
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
  const compiled: CompiledAssetGeometry = {
    ...(warnings.length ? { warnings } : {}),
    motion_data: { layers, graph_bytes: [] },
    ...(masks.length ? { masks } : {}),
    ...(compiledJumpZones.length
      ? { jump_zones: compiledJumpZones, jump_line_pairs: compiledJumpPairs }
      : {}),
    ...(lights.length
      ? {
          light_sectors: lights.flatMap((light) => {
            if (light.receivers || light.receiverSegments) {
              const segmentReceivers = (light.receiverSegments ?? []).flatMap((segment, index) => {
                const matches = areas.flatMap((area) => {
                  const point = lightReceiverIntersection(segment, area.plane);
                  return point && inside([point[0], point[1] - point[2]], area.polygon)
                    ? [{ area, point }]
                    : [];
                });
                if (new Set(matches.map(({ area }) => area.sector)).size !== 1) {
                  const message = `${light.id}: receiving segment ${index} must intersect exactly one walkable surface`;
                  if (!options.bestEffort) throw new Error(message);
                  warnings.push(`Light receiver omitted: ${message}.`);
                  return [];
                }
                return [matches[0]!.point];
              });
              const layers = new Set(
                [...(light.receivers ?? []), ...segmentReceivers].flatMap((point, index) => {
                  // These anchors select a layer and are not serialized as integer
                  // geometry. Rounding can move a valid interior anchor outside.
                  const projected: Point = [point[0], point[1] - point[2]];
                  if (!inside(projected, light.polygon))
                    throw new Error(
                      `${light.id}: receiver ${index} lies outside the light contour`,
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
            const invalid =
              endpoints.length < 2 || !endpoints.some((d) => d.door_type === 5)
                ? `Lift ${lift.id} needs at least two traversal doors including a low door`
                : new Set(endpoints.map((d) => d.point_out[1])).size < 2
                  ? `Lift ${lift.id} needs distinct projected endpoint heights after placement`
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
              doors: endpoints,
            };
          }),
        }
      : {}),
  };
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
