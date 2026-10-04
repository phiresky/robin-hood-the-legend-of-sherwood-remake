/** One-time authoring migration. Never imported by the editor's map compiler. */
import fs from "node:fs/promises";
import path from "node:path";
import { parseArgs } from "node:util";
import { createHash } from "node:crypto";
import { recoverReviewedMasks, type ReviewedMaskRecipe } from "./recover-reviewed-masks.ts";
import { maskReferenceResolver } from "../../shared/src/mask-references.ts";
import { recoverMaskStateLinks } from "./recover-mask-state-links.ts";
import {
  recoverReviewedProjections,
  type ReviewedProjections,
} from "./recover-reviewed-projections.ts";
import {
  recoverReviewedNavigationJoins,
  type ReviewedNavigationJoins,
} from "./recover-reviewed-navigation-joins.ts";
import polygonClipping, { type Polygon, type MultiPolygon } from "polygon-clipping";
import { applyAffineMatrix } from "../../shared/src/geometry.ts";
import {
  gameToScene,
  sceneToGame,
  partMatrix,
  transformedObstacle,
  type Level3DObject,
  type Vec3,
  type Point,
  type ProtoLevel,
} from "@rle/shared";
import { recoverEndpointElevation, distanceToPolygon } from "./recovery-elevation.ts";
import { readStoredMap, pinnedDescriptors } from "./stored-map.ts";
import { recoverGroundGameplay, polygonArea } from "./recover-ground-gameplay.ts";
import {
  recoverGroundReceivers,
  type ReviewedGroundReceivers,
} from "./recover-ground-receivers.ts";
import { partitionMovementObstacles } from "../../shared/src/partition-movement-obstacles.ts";
import {
  recoveredGameplayDefinition,
  descriptorGameplayPacket,
  type RecoveredGameplayPacket,
} from "./recovered-gameplay-definition.ts";
import type { AssetGameplay, GameplayAssetDescriptor } from "../../shared/src/asset-gameplay.ts";
import { diagnoseGameplayCandidates } from "./diagnose-gameplay-candidates.ts";
import {
  recoverSoundSource,
  containsSoundPolyline,
  uniqueSoundOwner,
  declaredSoundOwners,
} from "./recover-sound-source.ts";
import { recoverAuthoredSounds } from "./recover-authored-sounds.ts";
import { declaredLightOwners } from "./recover-light-owner.ts";
import { containsLightPolygon, recoverLightField } from "./recover-light-region.ts";
import { recoverJumpGeometry, recoverJumpSegment } from "./recover-jump-geometry.ts";
import { terrainOwnsJump } from "./terrain-jump-ownership.ts";
import { jumpEdgeOwners } from "./jump-edge-ownership.ts";
import { recoverMotionStates } from "./recover-motion-states.ts";
import { recoverMovementTransition } from "./recover-movement-transition.ts";
import { recoverAppearanceBindings } from "./recover-appearance-bindings.ts";
import {
  reviewedTransitionPlanes,
  type ReviewedTransitionPlanes,
} from "./reviewed-transition-planes.ts";
import { recoverLiftJoins } from "./recover-lift-joins.ts";
import {
  nonrenderingGameplayOwners,
  type GameplayOwnershipCatalog,
} from "./nonrendering-gameplay-owners.ts";
import { recoveryDoorGroups } from "./recovery-door-groups.ts";
import { declaredInteriorSources } from "./recovery-interior-sources.ts";
import {
  declaredDoorOwners,
  doorOwnershipFootprint,
  recoverDoorStateOwner,
  unownedInteriorEntrances,
} from "./recover-door-owner.ts";
import { quantizeGeneratedMotionPolygon } from "../../shared/src/motion-quantization.ts";
import { partitionRecoverySurfaces } from "./recovery-surface-partition.ts";
import { recoverSurfaceOwners } from "./recovery-surface-owners.ts";
import { recoverMovementClearance } from "./recover-movement-clearance.ts";
import { recoverWholeAssetVolume } from "./recover-whole-asset-volume.ts";
import { normalizeGameplayStateViews } from "../../shared/src/gameplay-state-views.ts";
import { declaredEndpointBindings, recoverDeclaredEndpoint } from "./recovery-endpoint-binding.ts";
import {
  heightPlane as fitHeightPlane,
  planeHeight as evaluateHeight,
  type HeightPlane,
} from "../../shared/src/gameplay-plane.ts";

const { values } = parseArgs({
  options: {
    map: { type: "string" },
    library: { type: "string", default: "../library" },
    source: { type: "string" },
    out: { type: "string" },
    ownership: { type: "string" },
    "mask-definitions": { type: "string" },
    "navigation-definitions": { type: "string" },
    "projection-definitions": { type: "string" },
    "ground-receivers": { type: "string" },
    "transition-planes": { type: "string" },
    "preserve-ground-boundaries": { type: "boolean", default: false },
    "precise-ground-ownership": { type: "boolean", default: false },
    "require-movement-coverage": { type: "boolean", default: false },
  },
});
if (!values.map || !values.source || !values.out)
  throw new Error(
    "Usage: --map <saved-map.json> --library <library> --source <proto-level.json> --out <authoring directory>",
  );
const inputDocument = await readStoredMap(values.map, values.library);
const inputDescriptors = await pinnedDescriptors(
  values.library,
  inputDocument.assetSources ?? [],
  inputDocument.sceneAssets,
);
const { document, descriptors } = normalizeGameplayStateViews(inputDocument, inputDescriptors);
const sourceBytes = await fs.readFile(values.source);
const proto: ProtoLevel = JSON.parse(sourceBytes.toString());
const planeDefinitions: ReviewedTransitionPlanes | undefined = values["transition-planes"]
  ? JSON.parse(await fs.readFile(values["transition-planes"], "utf8"))
  : undefined;
const transitionPlanes = planeDefinitions
  ? reviewedTransitionPlanes(
      proto,
      createHash("sha256").update(sourceBytes).digest("hex"),
      planeDefinitions,
    )
  : new Map<number, HeightPlane>();
const locals = new Map<
  number,
  {
    asset: string;
    node: string;
    part: Level3DObject;
    collisionId?: string;
    sourceShape?: ProtoLevel["sight_obstacles"][number];
  }[]
>();
for (const part of document.objects) {
  const match = /^asset:([^:]+):(.+)$/.exec(part.node);
  if (!match || part.source.obstacle === undefined) continue;
  if (descriptors.get(match[1]!)?.parts.find((p) => p.node === match[2])?.collision === "none")
    continue;
  const list = locals.get(part.source.obstacle) ?? [];
  if (!list.some((item) => item.asset === match[1] && item.node === match[2]))
    list.push({ asset: match[1]!, node: match[2]!, part });
  locals.set(part.source.obstacle, list);
}
const packets = new Map<
  string,
  RecoveredGameplayPacket & {
    version: 1;
    status: "needs-review";
    issues: string[];
    gameplayCandidate?: AssetGameplay;
  }
>();
const recoveredMaterials = new Set<number>();
const recoveredProjectionMaterials = new Set<number>();
const stateSightReferences = new Set(
  proto.patches.flatMap((patch) => [...patch.old_sight_obstacles, ...patch.new_sight_obstacles]),
);
const packet = (asset: string) => {
  let p = packets.get(asset);
  if (!p) {
    p = {
      version: 1,
      status: "needs-review",
      asset,
      surfaces: [],
      connections: [],
      issues: [],
    };
    packets.set(asset, p);
  }
  return p;
};
for (const descriptor of descriptors.values()) {
  if (descriptor.editor_usage === "map-background") continue;
  Object.assign(packet(descriptor.id), descriptorGameplayPacket(descriptor));
  packet(descriptor.id).issues.push(
    "Geometry seeded from asset parts; verify recovered movement clearances and feature coverage before publication",
  );
  if (descriptor.parts.some((p) => p.mission_profile))
    packet(descriptor.id).issues.push(
      "Preview part bounds do not establish navigation; author associated walkable surfaces, state transitions and behaviours separately",
    );
}
const localize = (part: Level3DObject, point: Vec3): Vec3 => {
  const m = partMatrix(document.camera, document, part),
    p = gameToScene(document.camera, ...point);
  const offset = p.map((v, i) => v - m[12 + i]!) as Vec3;
  // Placement transforms are rigid in the scene frame; transpose the rotation.
  const local = [0, 1, 2].map(
    (col) => m[col * 4]! * offset[0] + m[col * 4 + 1]! * offset[1] + m[col * 4 + 2]! * offset[2],
  ) as Vec3;
  return sceneToGame(document.camera, local);
};
const close = (points: Point[]): Polygon => [[...points, points[0]!]];
const planeHeight = (
  points: ProtoLevel["sight_obstacles"][number]["points"],
  x: number,
  y: number,
) => {
  points = points.map((p) => ({ ...p, y: p.y - p.z_top }));
  const a = points[0]!;
  for (let i = 1; i + 1 < points.length; i++) {
    const b = points[i]!,
      c = points[i + 1]!,
      det = (b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y);
    if (Math.abs(det) < 1e-6) continue;
    const dx = ((b.z_top - a.z_top) * (c.y - a.y) - (c.z_top - a.z_top) * (b.y - a.y)) / det;
    const dy = ((b.x - a.x) * (c.z_top - a.z_top) - (c.x - a.x) * (b.z_top - a.z_top)) / det;
    return a.z_top + (x - a.x) * dx + (y - a.y) * dy;
  }
  throw new Error("Projection surface has no valid plane");
};
const unresolved: unknown[] = [];
const ownershipPath =
  values.ownership ??
  new URL(
    `../../refinement/catalogs/${encodeURIComponent((document.sourceMap ?? path.basename(values.source).replace(/\.rhp\.json$/i, "")).toLowerCase())}.json`,
    import.meta.url,
  );
let ownership: GameplayOwnershipCatalog | undefined;
try {
  ownership = JSON.parse(await fs.readFile(ownershipPath, "utf8"));
} catch (error) {
  if (values.ownership || (error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
}
if (ownership)
  for (const entry of nonrenderingGameplayOwners(ownership, locals)) {
    if (locals.has(entry.source)) continue;
    if (!entry.owner) {
      unresolved.push({ kind: "nonrendering-owner", ...entry });
      continue;
    }
    const source = proto.sight_obstacles[entry.source];
    if (!source) throw new Error(`Missing non-rendering gameplay volume ${entry.source}`);
    const owner = entry.owner;
    const id = `gameplay-volume-${entry.source}`;
    const { projection_area: _projection, material_indices: _materials, ...flags } = source;
    const shape = {
      ...flags,
      points: source.points.map((p) => {
        const bottom = localize(owner.part, [p.x, p.y, p.z_bottom]);
        const top = localize(owner.part, [p.x, p.y, p.z_top]);
        if (Math.hypot(bottom[0] - top[0], bottom[1] - top[1]) > 1e-5)
          throw new Error(
            `Non-rendering gameplay volume ${entry.source} needs a vertical owner frame`,
          );
        return { x: top[0], y: top[1], z_bottom: bottom[2], z_top: top[2] };
      }),
    };
    (packet(owner.asset).volumes ??= []).push({ id, node: owner.node, shape });
    locals.set(entry.source, [{ ...owner, collisionId: id, sourceShape: source }]);
    packet(owner.asset).issues.push(
      `Non-rendering gameplay restored from explicit ownership: ${entry.declaredOwner}`,
    );
  }
for (const entry of ownership?.physical_volume_sources ?? []) {
  const descriptor = descriptors.get(entry.owner);
  const source = proto.sight_obstacles[entry.obstacle];
  const owners = locals.get(entry.obstacle) ?? [];
  const frames = owners.filter((owner) => owner.node === entry.node);
  const pin = inputDocument.assetSources?.find((asset) => asset.id === entry.owner);
  if (
    !descriptor ||
    !source ||
    frames.length !== 1 ||
    owners.some((owner) => owner.asset !== entry.owner) ||
    pin?.model_sha256 !== entry.model_sha256 ||
    createHash("sha256").update(sourceBytes).digest("hex") !== entry.source_sha256
  )
    throw new Error(`Whole-volume ownership or source pins changed: ${entry.owner}`);
  const owner = frames[0]!;
  const draft = packet(entry.owner);
  if (draft.collision !== undefined || draft.volumes?.length)
    throw new Error(`Whole-volume recovery would replace existing definitions: ${entry.owner}`);
  const recovered = recoverWholeAssetVolume({
    descriptor,
    source,
    sourceIndex: entry.obstacle,
    node: entry.node,
    localize: (point) => localize(owner.part, point),
  });
  Object.assign(draft, recovered);
  locals.set(entry.obstacle, [
    { ...owner, collisionId: recovered.volumes[0].id, sourceShape: source },
  ]);
  draft.issues.push(
    "Reviewed whole physical volume replaces visual component bounds; verify placement before publication",
  );
}
const coverage: unknown[] = [];
const movementStateInventory: {
  sector: number;
  layer: number;
  transitions: ReturnType<typeof recoverMotionStates>["transitions"];
}[] = [];
const groundReceiverDefinitions: ReviewedGroundReceivers | undefined = values["ground-receivers"]
  ? JSON.parse(await fs.readFile(values["ground-receivers"], "utf8"))
  : undefined;
const groundReceivers = new Map(
  (groundReceiverDefinitions
    ? recoverGroundReceivers(
        document,
        descriptors,
        proto,
        createHash("sha256").update(sourceBytes).digest("hex"),
        groundReceiverDefinitions,
        localize,
      )
    : []
  ).map((entry) => [entry.source_obstacle, entry]),
);
const groundAreas: Parameters<typeof recoverGroundGameplay>[0] = [];
const groundAreaSectors = new Set<number>();
let transferredGroundExclusions: MultiPolygon = [];
const clearanceSources: { regions: MultiPolygon; plane: HeightPlane }[] = [];
const groundProjectionOwners: {
  asset: string;
  node: string;
  part: Level3DObject;
  footprint: Point[];
}[] = [];
let sector = 0;
const sourceMotionAreas = new Map<number, { layer: number; polygon: Point[] }>();
for (const [layer, areas] of proto.motion_data.layers.entries())
  for (const rawMotion of areas) {
    const identity = sector;
    sourceMotionAreas.set(identity, { layer, polygon: rawMotion.polygon.points });
    sector += 1 + rawMotion.obstacles.length;
    const { base: motion, transitions } = recoverMotionStates(
      rawMotion,
      identity,
      layer,
      proto.patches,
    );
    if (transitions.length) {
      movementStateInventory.push({ sector: identity, layer, transitions });
    }
    const supports = proto.sight_obstacles.flatMap((obstacle, index) =>
      Array.isArray(obstacle.projection_area) &&
      obstacle.projection_area[0] === identity &&
      obstacle.projection_area[1] === layer
        ? [{ obstacle, index }]
        : [],
    );
    if (layer === 0 && supports.length && !motion.is_lift) {
      const raised = supports.filter(({ obstacle }) =>
        obstacle.points.some((p) => Math.abs(p.z_top) > 1e-4),
      );
      if (
        !motion.is_lift &&
        motion.state_id === 0 &&
        motion.obstacles.every((o) => o.state_id === 0) &&
        raised.every(({ index }) => locals.get(index)?.length === 1)
      ) {
        const exclusions = raised
          .filter(({ index }) => !groundReceivers.has(index))
          .map(({ obstacle, index }) => {
            const footprint = obstacle.points.map((p): Point => [p.x, p.y - p.z_top]);
            groundProjectionOwners.push({ ...locals.get(index)![0]!, footprint });
            return { polygon: { points: footprint } };
          });
        groundAreas.push({
          polygon: motion.polygon,
          obstacles: [...motion.obstacles, ...exclusions],
        });
        groundAreaSectors.add(identity);
      } else {
        unresolved.push({
          kind: "terrain-projection-remainder",
          sector: identity,
          layer,
          reason: "Ground remainder requires static geometry and unique projection-surface owners",
        });
      }
    }
    if (!supports.length) {
      if (
        layer === 0 &&
        !motion.is_lift &&
        motion.state_id === 0 &&
        motion.obstacles.every((o) => o.state_id === 0)
      ) {
        groundAreas.push(motion);
        groundAreaSectors.add(identity);
        clearanceSources.push({
          regions: motion.obstacles.length
            ? polygonClipping.difference(
                close(motion.polygon.points),
                ...motion.obstacles.map((o) => close(o.polygon.points)),
              )
            : [close(motion.polygon.points)],
          plane: [0, 0, 0],
        });
        continue;
      }
      unresolved.push({
        kind: "terrain-motion",
        sector: identity,
        layer,
        reason:
          "No projection-surface owner; separate terrain boundaries from placed-asset cutouts before authoring",
      });
      continue;
    }
    let recoveredArea = 0;
    let quantizationDifferenceArea = 0;
    // Preserve a continuous local walking region independently of its height planes.
    // Multi-asset ownership needs explicit joins; a shared label cannot cross placements.
    const supportOwners = supports.map(({ index }) => locals.get(index) ?? []);
    const soleOwner = supportOwners[0]?.[0]?.asset;
    const regionIsLocal =
      !motion.is_lift &&
      supports.length > 0 &&
      soleOwner !== undefined &&
      supportOwners.every((owners) => owners.length === 1 && owners[0]!.asset === soleOwner);
    const navigationRegion = regionIsLocal ? `walk-region-${identity}` : undefined;
    const staticMotion = motion.state_id === 0 && motion.obstacles.every((o) => o.state_id === 0);
    const preserveSingleLiftBoundary =
      motion.is_lift && staticMotion && supports.length === 1 && supportOwners[0]!.length === 1;
    const partition = partitionRecoverySurfaces(
      close(motion.polygon.points),
      motion.obstacles.map((o) => close(o.polygon.points)),
      supports.map(({ obstacle }) => ({
        polygon: close(obstacle.points.map((p) => [p.x, p.y - p.z_top])),
        maximumHeight: Math.max(...obstacle.points.map((p) => Math.max(p.z_top, p.z_bottom))),
      })),
      preserveSingleLiftBoundary,
    );
    if (layer === 0 && staticMotion) {
      // Shared receivers retain ground navigation beneath their receiving footprint.
      // Include that coverage when recovering clearances for nearby collision parts.
      const groundCoverage = [
        ...partition.ground,
        ...supports.flatMap(({ index }, supportIndex) =>
          groundReceivers.has(index) ? partition.surfaces[supportIndex]! : [],
        ),
      ];
      clearanceSources.push({
        regions: groundCoverage.length ? polygonClipping.union(groundCoverage) : [],
        plane: [0, 0, 0],
      });
    }
    for (const [supportIndex, { obstacle, index }] of supports.entries()) {
      const binding = groundReceivers.get(index);
      if (binding) {
        const target = packet(binding.asset);
        (target.projectionReceivers ??= []).push({
          id: `${binding.node}-receiver`,
          node: binding.node,
          volume: binding.node,
          anchor: binding.localAnchor,
        });
        for (const [materialIndex, material] of obstacle.material_indices.entries()) {
          const source = proto.material_sectors[material];
          if (!source) throw new Error(`Missing material region ${material}`);
          (target.materials ??= []).push({
            id: `${binding.node}-receiver-material-${materialIndex}`,
            node: binding.node,
            material: source.material,
            ground: false,
            obstacles: [binding.node],
            polygon: source.polygon.points.map(([x, y]) => {
              const z = planeHeight(obstacle.points, x, y);
              return localize(locals.get(index)![0]!.part, [x, y + z, z]);
            }),
          });
          recoveredMaterials.add(material);
        }
        recoveredProjectionMaterials.add(index);
        recoveredArea += polygonArea(partition.surfaces[supportIndex]!);
        continue;
      }
      const owners = locals.get(index) ?? [];
      if (!owners.length) {
        unresolved.push({
          kind: "surface-owner",
          sector: identity,
          layer,
          obstacle: index,
          candidates: owners.map((o) => o.asset),
        });
        continue;
      }
      let regions = preserveSingleLiftBoundary
        ? [close(motion.polygon.points)]
        : polygonClipping.intersection(
            close(motion.polygon.points),
            close(obstacle.points.map((p) => [p.x, p.y - p.z_top])),
          );
      if (motion.obstacles.length)
        regions = polygonClipping.difference(
          regions,
          ...motion.obstacles.map((o) => close(o.polygon.points)),
        );
      if (staticMotion)
        clearanceSources.push({
          // Joined traversal planes share one movement area. Projection
          // priority must not carve collision seams between their supports.
          regions: motion.is_lift ? regions : partition.surfaces[supportIndex]!,
          plane: fitHeightPlane(
            obstacle.points.map((p) => [p.x, p.y - p.z_top, p.z_top]),
            false,
          ),
        });
      const overlapArea = polygonArea(regions) - polygonArea(partition.surfaces[supportIndex]!);
      if (overlapArea > 1e-6) {
        unresolved.push({
          kind: "projection-priority",
          sector: identity,
          layer,
          obstacle: index,
          overlapArea,
          reason:
            "Overlapping surfaces need placement-time priority; do not freeze another asset's footprint into this surface",
        });
      }
      let owned = [regions];
      if (owners.length > 1) {
        const split = recoverSurfaceOwners(
          regions,
          owners.map((owner) => {
            const definition = descriptors
              .get(owner.asset)!
              .parts.find((p) => p.node === owner.node);
            if (!definition?.obstacle_local_game) return [];
            const shape = transformedObstacle(document, {
              ...owner.part,
              obstacle: definition.obstacle_local_game,
            });
            return close(shape.points.map((p) => [p.x, p.y - p.z_top]));
          }),
          true,
        );
        owned = split.owned;
        for (const owner of owners) packet(owner.asset).issues.push(...split.warnings);
        coverage.push({
          kind: "split-surface-ownership",
          obstacle: index,
          candidates: owners.map((o) => ({ asset: o.asset, node: o.node })),
          unresolvedArea: split.unresolvedArea,
        });
        if (split.unresolvedArea > 1e-6)
          unresolved.push({
            kind: "surface-owner",
            sector: identity,
            layer,
            obstacle: index,
            unresolvedArea: split.unresolvedArea,
            candidates: owners.map((o) => o.asset),
          });
      }
      for (const [ownerIndex, owner] of owners.entries()) {
        for (const [regionIndex, generated] of owned[ownerIndex]!.entries()) {
          const region = quantizeGeneratedMotionPolygon(
            generated,
            Math.round,
            `${owner.node}-walk-${regionIndex}`,
            packet(owner.asset).issues,
          );
          quantizationDifferenceArea += region
            ? polygonArea(polygonClipping.xor(generated, region))
            : polygonArea([generated]);
          if (!region) continue;
          const points = region[0]!.slice(0, -1).map(([x, y]) => [x, y] as Point);
          recoveredArea += polygonArea([region]);
          const vertices = points.map(([x, y]) =>
            localize(owner.part, [
              x,
              y + planeHeight(obstacle.points, x, y),
              planeHeight(obstacle.points, x, y),
            ]),
          );
          const surfaceId = `${owner.collisionId ?? owner.node}-walk-${regionIndex}`;
          const projectionVolume =
            owners.length === 1 &&
            (stateSightReferences.has(index) || (motion.is_lift && owner.collisionId !== undefined))
              ? (owner.collisionId ?? owner.node)
              : undefined;
          const materialRegions = obstacle.material_indices.map((material, materialIndex) => {
            const source = proto.material_sectors[material];
            if (!source) throw new Error(`Missing material region ${material}`);
            const id = `${surfaceId}-material-${materialIndex}`;
            (packet(owner.asset).materials ??= []).push({
              id,
              node: owner.node,
              material: source.material,
              ground: false,
              obstacles: projectionVolume === undefined ? [] : [projectionVolume],
              polygon: source.polygon.points.map(([x, y]) => {
                const z = planeHeight(obstacle.points, x, y);
                return localize(owner.part, [x, y + z, z]);
              }),
            });
            recoveredMaterials.add(material);
            return id;
          });
          recoveredProjectionMaterials.add(index);
          packet(owner.asset).surfaces.push({
            id: surfaceId,
            node: owner.node,
            navigationRegion,
            ...(projectionVolume === undefined
              ? {
                  projectionMaterials: {
                    defaultMaterial: obstacle.default_material,
                    regions: materialRegions,
                    planePoints: [
                      obstacle.points[1]!,
                      obstacle.points[2]!,
                      obstacle.points[0]!,
                    ].map((point) => localize(owner.part, [point.x, point.y, point.z_top])) as [
                      Vec3,
                      Vec3,
                      Vec3,
                    ],
                    priority: -index,
                    footprint:
                      owners.length === 1
                        ? obstacle.points.map((point) =>
                            localize(owner.part, [point.x, point.y, point.z_top]),
                          )
                        : descriptors
                            .get(owner.asset)!
                            .parts.find((part) => part.node === owner.node)!
                            .obstacle_local_game!.points.map((point): Vec3 => [
                              point.x,
                              point.y,
                              point.z_top,
                            ]),
                    priorityHeight: localize(owner.part, [
                      0,
                      0,
                      Math.max(
                        ...obstacle.points.map((point) => Math.max(point.z_top, point.z_bottom)),
                      ),
                    ])[2],
                  },
                }
              : { projectionVolume }),
            vertices,
            kind: motion.is_lift ? "lift" : "walkable",
            holes: region
              .slice(1)
              .map((hole) =>
                hole
                  .slice(0, -1)
                  .map(([x, y]) =>
                    localize(owner.part, [
                      x,
                      y + planeHeight(obstacle.points, x, y),
                      planeHeight(obstacle.points, x, y),
                    ]),
                  ),
              ),
          });
          if (motion.is_lift)
            packet(owner.asset).issues.push(
              "Review lift surface coverage and endpoint ownership before publishing",
            );
        }
      }
    }
    coverage.push({
      sector: identity,
      layer,
      sourceArea: polygonArea(
        motion.obstacles.length
          ? polygonClipping.difference(
              close(motion.polygon.points),
              ...motion.obstacles.map((o) => close(o.polygon.points)),
            )
          : [close(motion.polygon.points)],
      ),
      recoveredArea,
      quantizationDifferenceArea,
      uncoveredArea: polygonArea(partition.ground),
    });
  }
if (groundAreas.length) {
  const grounds = document.sceneAssets.filter((s) => s.role === "ground");
  if (grounds.length !== 1) {
    unresolved.push({ kind: "terrain-owner", candidates: grounds.map((s) => s.id) });
  } else {
    const owners = [...locals]
      .flatMap(([index, candidates]) => {
        const obstacle = proto.sight_obstacles[index];
        if (
          candidates.length !== 1 ||
          !obstacle?.solid ||
          !obstacle.points.every((p) => p.z_bottom <= 0 && p.z_top > 0)
        )
          return [];
        return [{ ...candidates[0]!, footprint: obstacle.points.map((p): Point => [p.x, p.y]) }];
      })
      .concat(groundProjectionOwners);
    const ground = recoverGroundGameplay(
      groundAreas,
      owners,
      values["preserve-ground-boundaries"],
      values["precise-ground-ownership"],
    );
    transferredGroundExclusions = ground.blockers.flatMap((b) => b.regions);
    const terrain = packet(grounds[0]!.id);
    terrain.issues.push(...ground.warnings);
    for (const section of ground.sections) {
      if (values["preserve-ground-boundaries"]) {
        const holes = section.movementContours.flatMap((contour) =>
          contour.regions
            .flatMap((region) => partitionMovementObstacles(region, true))
            .map((points) => ({ id: `${grounds[0]!.id}/${contour.id}`, points })),
        );
        terrain.surfaces.push({
          id: `${section.navigationRegion}-0`,
          preserveMovementPrecision: true,
          preserveMovementBoundary: true,
          navigationRegion: section.navigationRegion,
          node: "$root",
          kind: "walkable",
          vertices: section.movementBoundary.map(([x, y]) => [x, y, 0]),
          holes: holes.map((hole) => hole.points.map(([x, y]) => [x, y, 0])),
          holeContours: holes.map((hole) => hole.id),
        });
      } else
        for (const [index, region] of section.terrain.entries())
          terrain.surfaces.push({
            id: `${section.navigationRegion}-${index}`,
            preserveMovementPrecision: true,
            navigationRegion: section.navigationRegion,
            node: "$root",
            kind: "walkable",
            vertices: region[0]!.slice(0, -1).map(([x, y]) => [x, y, 0]),
            holes: region.slice(1).map((hole) => hole.slice(0, -1).map(([x, y]) => [x, y, 0])),
          });
    }
    for (const [index, blocker] of ground.blockers.entries()) {
      const owner = owners.find((o) => o.asset === blocker.asset && o.node === blocker.node)!;
      const ownedBlockers = (packet(owner.asset).movementBlockers ??= []);
      const contours = blocker.contours ?? [{ id: undefined, regions: blocker.regions }];
      for (const [contourIndex, contour] of contours.entries())
        for (const [regionIndex, region] of contour.regions.entries()) {
          const local = (ring: Point[]) =>
            ring.slice(0, -1).map(([x, y]) => localize(owner.part, [x, y, 0]));
          ownedBlockers.push({
            preserveMovementPrecision: true,
            id: `${owner.node}-ground-blocker-${index}-${contour.id === undefined ? "" : `${contourIndex}-`}${regionIndex}`,
            ...(contour.id === undefined
              ? {}
              : { movementContour: `${grounds[0]!.id}/${contour.id}` }),
            node: owner.node,
            vertices: local(region[0]!),
            holes: region.slice(1).map(local),
          });
        }
      packet(owner.asset).issues.push(
        "Review movement contour ownership: footprint intersections can split exclusions shared by adjacent assets",
      );
    }
    terrain.issues.push(
      "Review residual terrain exclusions and asset coverage; geometric round-trip equality does not establish ownership",
    );
    coverage.push({
      kind: "ground-decomposition",
      preservedBoundaries: values["preserve-ground-boundaries"],
      preciseOwnershipIntersections: values["precise-ground-ownership"],
      sourceArea: ground.sourceArea,
      recoveredArea: ground.reconstructedArea,
      differenceArea: ground.differenceArea,
      coordinateGrid: ground.coordinateGrid,
      blockerOwners: ground.blockers.filter((blocker) => blocker.regions.length).length,
      authoredMovementOwners: ground.blockers.length,
      navigationRegions: ground.sections.map(({ navigationRegion, differenceArea }) => ({
        navigationRegion,
        differenceArea,
      })),
    });
  }
}
let navigationJoinRecovery: { asset: string; surface: string; region: string; edges: number }[] =
  [];
if (values["navigation-definitions"]) {
  const definitions: ReviewedNavigationJoins = JSON.parse(
    await fs.readFile(values["navigation-definitions"], "utf8"),
  );
  const updates = recoverReviewedNavigationJoins(
    document,
    proto,
    createHash("sha256").update(sourceBytes).digest("hex"),
    definitions,
    packets,
  );
  navigationJoinRecovery = updates.map(
    ({
      asset,
      surface,
      region,
      edges,
      heightTolerance,
      vertices,
      holes,
      preserveMovementBoundary,
    }) => {
      surface.navigationRegion = region;
      surface.navigationJoins = edges.length ? edges : undefined;
      surface.navigationJoinHeightTolerance = heightTolerance;
      if (preserveMovementBoundary !== undefined)
        surface.preserveMovementBoundary = preserveMovementBoundary;
      if (vertices) {
        surface.vertices = vertices;
        surface.holes = holes ?? [];
        const part = document.objects.find((p) => p.node === `asset:${asset}:${surface.node}`)!;
        const matrix = partMatrix(document.camera, document, part);
        const placed = (points: Vec3[]) =>
          points.map((p) => {
            const [x, y, z] = sceneToGame(
              document.camera,
              applyAffineMatrix(matrix, gameToScene(document.camera, ...p)),
            );
            return [x, y - z, z] as Vec3;
          });
        const outer = placed(vertices);
        const boundary = close(outer.map(([x, y]) => [x, y]));
        // These surfaces already carry explicit exclusions. Clear derived solid
        // slices across the whole boundary so rounded duplicates cannot expand
        // an authored exclusion or leave a false seam between receiving planes.
        clearanceSources.push({
          regions: [boundary],
          plane: fitHeightPlane(outer),
        });
      }
      return { asset, surface: surface.id, region, edges: edges.length };
    },
  );
}
// Restore openings only in each placed asset's own derived collision. These
// local contours move with the asset; no assembled-map override is exported.
for (const [sourceIndex, source] of clearanceSources.entries()) {
  for (const owners of locals.values())
    for (const owner of owners) {
      if (packet(owner.asset).movementBlockers !== undefined) continue;
      const definition = descriptors.get(owner.asset)!.parts.find((p) => p.node === owner.node);
      const localObstacle = owner.sourceShape ?? definition?.obstacle_local_game;
      if (!localObstacle?.solid) continue;
      const solid =
        owner.sourceShape ??
        transformedObstacle(document, {
          ...owner.part,
          obstacle: localObstacle,
        });
      let regions: MultiPolygon;
      try {
        regions = recoverMovementClearance(source.regions, source.plane, solid, 1);
      } catch (error) {
        unresolved.push({
          kind: "movement-clearance",
          asset: owner.asset,
          node: owner.node,
          source: sourceIndex,
          error: String(error),
        });
        continue;
      }
      for (const [regionIndex, region] of regions.entries()) {
        const id = `${owner.collisionId ?? owner.node}-clearance-${sourceIndex}-${regionIndex}`;
        const local = (ring: Point[]) =>
          ring.slice(0, -1).map(([x, y]) => {
            const z = evaluateHeight(source.plane, [x, y]);
            return localize(owner.part, [x, y + z, z]);
          });
        (packet(owner.asset).movementClearances ??= []).push({
          id,
          node: owner.node,
          vertices: local(region[0]!),
          holes: region.slice(1).map(local),
        });
      }
    }
}
// Recover lifts only where their surface has a unique owner. Neighbour endpoints
// remain geometric queries; no source sector or layer indices enter asset packets.
const heightAt = (sector: number, layer: number, point: Point, projectionPoint = point) => {
  const supports = proto.sight_obstacles.filter(
    (o) =>
      Array.isArray(o.projection_area) &&
      o.projection_area[0] === sector &&
      o.projection_area[1] === layer,
  );
  return recoverEndpointElevation(
    supports.map((o) => ({
      distance: distanceToPolygon(
        projectionPoint,
        o.points.map((p) => [p.x, p.y - p.z_top]),
      ),
      height: planeHeight(o.points, ...point),
      maximumHeight: Math.max(...o.points.map((p) => Math.max(p.z_top, p.z_bottom))),
    })),
    layer === 0,
  );
};
const localEndpoint = (
  part: Level3DObject,
  point: Point,
  sector: number,
  layer: number,
  projectionPoint = point,
) => {
  const z = heightAt(sector, layer, point, projectionPoint);
  return localize(part, [point[0], point[1] + z, z]);
};
const sourceDoorCount = proto.buildings.reduce<number>(
  (sum, entry) =>
    sum +
    recoveryDoorGroups(
      entry as {
        Building?: { doors: SourceDoor[] };
        StandaloneDoors?: { doors: SourceDoor[] };
      },
    ).reduce((count, group) => count + group.doors.length, 0),
  0,
);
const endpointBindings = declaredEndpointBindings(
  ownership?.endpoint_bindings ?? [],
  sourceDoorCount,
  proto.patches.length,
);
const endpointBinding = (key: string, sector: number, layer: number, point: Point) => {
  const declaration = endpointBindings.get(key);
  return declaration
    ? recoverDeclaredEndpoint(
        declaration,
        point,
        layer,
        proto.sight_obstacles,
        (anchor) => {
          const area = sourceMotionAreas.get(sector);
          if (!area || area.layer !== layer || distanceToPolygon(anchor, area.polygon) !== 0)
            throw new Error("Declared receiving anchor must lie in its linked movement area");
          return heightAt(sector, layer, anchor);
        },
        (projection, point) => planeHeight(projection.points, ...point),
      )
    : { height: heightAt(sector, layer, point), anchor: undefined };
};
const movementTransitionRecovery: {
  patch: number;
  sector: number;
  layer: number;
  pair: number;
  asset: string;
  transition: string;
}[] = [];
for (const area of movementStateInventory)
  for (const change of area.transitions) {
    try {
      if (change.patches.length !== 1)
        throw new Error("Changing contours need one explicit patch owner");
      const source = proto.patches[change.patches[0]!]!;
      const refs = [...source.old_sight_obstacles, ...source.new_sight_obstacles];
      const owners = refs.map((ref) => {
        const matches = locals.get(ref) ?? [];
        if (matches.length !== 1) throw new Error(`Missing or ambiguous sight owner ${ref}`);
        return matches[0]!;
      });
      const declared = (ownership?.movement_transitions ?? []).filter(
        (entry) => entry.patch === change.patches[0],
      );
      if (declared.length > 1) throw new Error("Ambiguous explicit movement-transition ownership");
      if (declared[0]) {
        const { owner: asset, node } = declared[0];
        const parts = document.objects.filter((part) => part.node === `asset:${asset}:${node}`);
        if (parts.length !== 1 || !descriptors.get(asset)?.parts.some((part) => part.node === node))
          throw new Error("Explicit movement owner needs one pinned asset frame");
        owners.unshift({ asset, node, part: parts[0]! });
      }
      if (!owners.length)
        throw new Error("No sight ownership; explicit asset authoring is required");
      const owner = owners[0]!;
      if (owners.some((entry) => entry.asset !== owner.asset))
        throw new Error("Changing sight geometry spans assets; author independent local states");
      const p = packet(owner.asset);
      const descriptor = descriptors.get(owner.asset)!;
      const localRef = (index: number) => {
        const entry = locals.get(index)![0]!;
        return entry.collisionId ?? entry.node;
      };
      const initialSight = source.old_sight_obstacles.map(localRef);
      const appliedSight = source.new_sight_obstacles.map(localRef);
      const controlled = new Set([...initialSight, ...appliedSight]);
      const waypoint = endpointBinding(
        `patch-waypoint/${change.patches[0]}`,
        source.sector,
        source.layer,
        source.waypoint,
      );
      const definition = recoverMovementTransition({
        id: `movement-change-${change.patches[0]}`,
        node: owner.node,
        patch: source,
        initial: change.initial,
        applied: change.applied,
        initialSight,
        appliedSight,
        uncoveredPlane: transitionPlanes.get(change.patches[0]!),
        // Shared physical receivers retain the ground navigation plane for states.
        receivers: proto.sight_obstacles.filter(
          (obstacle, index) =>
            !groundReceivers.has(index) &&
            Array.isArray(obstacle.projection_area) &&
            obstacle.projection_area[0] === area.sector &&
            obstacle.projection_area[1] === area.layer,
        ),
        groundLayer: area.layer === 0,
        waypointHeight: waypoint.height,
        localize: (point) => localize(owner.part, point),
      });
      if (waypoint.anchor) definition.waypointAnchor = localize(owner.part, waypoint.anchor);
      // Keep permanent solids and their clearances, excluding both changing endpoints.
      // Explicit stable contours already replace part-derived movement collision.
      if (p.movementBlockers === undefined) {
        const solids = p.movementSolids ?? [
          ...descriptor.parts
            .filter((part) => part.obstacle_local_game?.solid)
            .map((part) => part.node),
          ...(p.volumes ?? []).filter((volume) => volume.shape.solid).map((volume) => volume.id),
        ];
        p.movementSolids = solids.filter((ref) => !controlled.has(ref));
      }
      (p.movementTransitions ??= []).push(definition);
      p.issues.push(
        "Movement/sight states recovered; visual states and effects still need separate authoring",
      );
      movementTransitionRecovery.push({
        patch: change.patches[0]!,
        sector: area.sector,
        layer: area.layer,
        pair: change.pair,
        asset: owner.asset,
        transition: definition.id,
      });
    } catch (error) {
      unresolved.push({
        kind: "movement-states",
        sector: area.sector,
        layer: area.layer,
        pair: change.pair,
        reason: String(error),
      });
    }
  }
for (const [index, lift] of proto.lifts.entries()) {
  const supports = proto.sight_obstacles.flatMap((obstacle, support) =>
    Array.isArray(obstacle.projection_area) &&
    obstacle.projection_area[0] === lift.motion_area_index
      ? [{ obstacle, owners: locals.get(support) ?? [] }]
      : [],
  );
  if (!supports.length || supports.some((s) => s.owners.length !== 1)) {
    unresolved.push({
      kind: "lift-owner",
      lift: index,
      candidates: supports.flatMap((s) => s.owners.map((o) => o.asset)),
    });
    continue;
  }
  try {
    const joins = recoverLiftJoins(
      supports.map((s) => s.obstacle.points.map((p): Vec3 => [p.x, p.y, p.z_top])),
    );
    const endpointOwners = (lift.doors as SourceDoor[]).map((door) => {
      const matches = supports
        .map((s, i) => ({
          i,
          distance: distanceToPolygon(
            door.point_in,
            s.obstacle.points.map((p): Point => [p.x, p.y - p.z_top]),
          ),
        }))
        .filter((p) => p.distance === 0);
      if (matches.length !== 1)
        throw new Error("Lift endpoint does not have one supporting segment");
      return matches[0]!.i;
    });
    for (const [supportIndex, support] of supports.entries()) {
      const owner = support.owners[0]!;
      const supportId = owner.collisionId ?? owner.node;
      const doors = (lift.doors as SourceDoor[]).flatMap((door, i) =>
        endpointOwners[i] !== supportIndex
          ? []
          : [
              {
                id: `${supportId}-endpoint-${i}`,
                node: owner.node,
                polygon: door.door_sector.points.map((point) => {
                  const z = heightAt(door.sector_out, door.layer_out, door.point_out);
                  const local = localize(owner.part, [point[0], point[1] + z, z]);
                  return [local[0], local[1]] as Point;
                }),
                inside: localEndpoint(owner.part, door.point_in, door.sector_in, door.layer_in),
                outside: localEndpoint(owner.part, door.point_out, door.sector_out, door.layer_out),
                // A transition midpoint can sit just outside its projection polygon.
                // Keep the plane selected by the actual inside endpoint.
                middle: localEndpoint(
                  owner.part,
                  door.point_mid,
                  door.sector_in,
                  door.layer_in,
                  door.point_in,
                ),
                type: door.door_type,
                locked: door.locked_pc,
                unlockable: door.unlockable,
                active: door.active,
                lockedVillains: door.locked_npc_villain,
                lockedCivilians: door.locked_npc_civilian,
                afterTransition: {
                  player: door.locked_pc_after_patch,
                  unlockable: door.unlockable_after_patch,
                  villains: door.locked_npc_villain_after_patch,
                  civilians: door.locked_npc_civilian_after_patch,
                },
              },
            ],
      );
      packet(owner.asset).connections.push({
        id: `${supportId}-lift`,
        node: owner.node,
        kind: "lift",
        surface: `${supportId}-walk-0`,
        type: lift.lift_type,
        direction: (() => {
          const angle = (lift.direction * Math.PI) / 8;
          const origin = localize(owner.part, [0, 0, 0]);
          const tip = localize(owner.part, [Math.sin(angle), -Math.cos(angle), 0]);
          return [tip[0] - origin[0], tip[1] - origin[1]] as Point;
        })(),
        ...(joins[supportIndex]!.length
          ? { joins: joins[supportIndex]!.map((p) => localize(owner.part, p)) }
          : {}),
        endpoints: doors,
      });
    }
  } catch (error) {
    unresolved.push({ kind: "lift-endpoint", lift: index, reason: String(error) });
  }
}
// Building ownership must be spatially unambiguous; do not guess from asset names.
type SourceDoor = {
  point_in: Point;
  point_out: Point;
  point_mid: Point;
  sector_in: number;
  sector_out: number;
  layer_in: number;
  layer_out: number;
  door_sector: { points: Point[] };
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
};
let recoveredBuildings = 0;
const doorStateOwnershipRecovery: {
  building: number;
  doors: number[];
  asset: string;
  connection: string;
}[] = [];
const recoveredDoors = new Map<
  number,
  { asset: string; id: string; node: string; part: Level3DObject }
>();
let doorOffset = 0;
const ownerFrames = (asset: string, node: string) =>
  descriptors.get(asset)?.parts.some((part) => part.node === node)
    ? document.objects
        .filter((part) => part.node === `asset:${asset}:${node}`)
        .map((part) => ({ asset, node, part }))
    : [];
const declaredDoors = declaredDoorOwners(
  ownership?.door_sources ?? [],
  sourceDoorCount,
  ownerFrames,
);
let roomOffset = 0;
const sourceRooms = new Map<number, number[]>();
for (const [index, entry] of proto.buildings.entries()) {
  const groups = recoveryDoorGroups(
    entry as { Building?: { doors: SourceDoor[] }; StandaloneDoors?: { doors: SourceDoor[] } },
  );
  const indices = groups.flatMap((group) => group.doors).map(() => roomOffset++);
  if (groups[0]?.kind === "building-interior") sourceRooms.set(index, indices);
}
const interiorSources = declaredInteriorSources(
  ownership?.interior_sources ?? [],
  sourceRooms,
  ownerFrames,
);
for (const pieces of interiorSources.values())
  if (pieces.some((piece) => piece.doors.some((door) => declaredDoors.has(door))))
    throw new Error("Shared interior declaration conflicts with ordinary door ownership");
for (const pieces of interiorSources.values())
  for (const piece of pieces) {
    const shapes = document.objects
      .filter((part) => part.obstacle && part.node.startsWith(`asset:${piece.owner}:`))
      .map((part) => transformedObstacle(document, part));
    for (const join of piece.joins)
      if (
        !shapes.some(
          (shape) =>
            shape.solid &&
            doorOwnershipFootprint(shape, join.point[2]).some(
              (polygon) => distanceToPolygon([join.point[0], join.point[1]], polygon) <= 1,
            ),
        )
      )
        throw new Error(
          `Shared interior socket must touch its asset's wall geometry: ${piece.owner}`,
        );
  }
for (const [index, entry] of proto.buildings.entries()) {
  const building = entry as {
    Building?: { doors: SourceDoor[] };
    StandaloneDoors?: { doors: SourceDoor[] };
  };
  const groups = recoveryDoorGroups(building);
  const doorIndices = new Map(
    groups.flatMap((group) => group.doors).map((door) => [door, doorOffset++] as const),
  );
  if (!groups.length) {
    recoveredBuildings++;
    continue;
  }
  if (groups.some((g) => !g.doors.length)) {
    unresolved.push({ kind: "building-empty", building: index });
    continue;
  }
  let recovered = 0;
  const recoveryGroups =
    interiorSources.get(index)?.map((piece) => ({
      ...groups[0]!,
      doors: groups[0]!.doors.filter((door) => piece.doors.includes(doorIndices.get(door)!)),
      authoring: piece,
    })) ?? groups.map((group) => ({ ...group, authoring: undefined }));
  for (const connection of recoveryGroups) {
    const { doors, sourceDoor } = connection;
    const isInterior = connection.kind === "building-interior";
    try {
      const declared = doors.map((door) => declaredDoors.get(doorIndices.get(door)!));
      const explicitOwner =
        connection.authoring?.frame ?? declared.find((owner) => owner !== undefined);
      if (
        !connection.authoring &&
        explicitOwner &&
        declared.some((owner) => owner?.asset !== explicitOwner.asset)
      )
        throw new Error("Interior door ownership must cover the whole room with one asset");
      const candidates = new Map<
        string,
        { owner: { asset: string; node: string; part: Level3DObject }; distance: number }
      >();
      const raisedCandidates: typeof candidates = new Map();
      const first = doors[0];
      if (first) {
        const z = isInterior
          ? endpointBinding(
              `door-outside/${doorIndices.get(first)!}`,
              first.sector_out,
              first.layer_out,
              first.point_out,
            ).height
          : endpointBinding(
              `door-inside/${doorIndices.get(first)!}`,
              first.sector_in,
              first.layer_in,
              first.point_in,
            ).height;
        for (const [obstacleIndex, owners] of locals) {
          if (owners.length !== 1) continue;
          const obstacle = proto.sight_obstacles[obstacleIndex]!;
          if (!obstacle.solid) continue;
          if (Math.min(...obstacle.points.map((p) => p.z_bottom)) > z + 24) continue;
          const distance = distanceToPolygon(
            [first.point_in[0], first.point_in[1] + z],
            obstacle.points.map((p) => [p.x, p.y]),
          );
          const owner = owners[0]!,
            previous = candidates.get(owner.asset);
          if (!previous || distance < previous.distance)
            candidates.set(owner.asset, { owner, distance });
          const footprints = doorOwnershipFootprint(obstacle, z);
          if (footprints.length) {
            const raisedDistance = Math.min(
              ...footprints.map((footprint) =>
                distanceToPolygon([first.point_in[0], first.point_in[1] + z], footprint),
              ),
            );
            const previousRaised = raisedCandidates.get(owner.asset);
            if (!previousRaised || raisedDistance < previousRaised.distance)
              raisedCandidates.set(owner.asset, { owner, distance: raisedDistance });
          }
        }
      }
      const ranked = [...candidates.values()].sort((a, b) => a.distance - b.distance);
      const choose = (entries: typeof ranked) =>
        entries[0] &&
        entries[0].distance <= 24 &&
        (!entries[1] || entries[1].distance - entries[0].distance >= 8)
          ? entries[0].owner
          : undefined;
      const spatialOwner =
        choose(ranked) ??
        choose([...raisedCandidates.values()].sort((a, b) => a.distance - b.distance));
      const stateOwner = recoverDoorStateOwner(
        doors.map((door) => doorIndices.get(door)!),
        proto.patches,
        locals,
      );
      if (!explicitOwner && !stateOwner && !spatialOwner) {
        unresolved.push({
          kind: "building-owner",
          building: index,
          sourceDoor,
          candidates: ranked
            .slice(0, 4)
            .map((c) => ({ asset: c.owner.asset, distance: c.distance })),
        });
        continue;
      }
      if (explicitOwner && stateOwner && explicitOwner.asset !== stateOwner.asset)
        throw new Error("Declared door owner conflicts with linked state geometry");
      const owner = explicitOwner ?? stateOwner ?? spatialOwner!;
      if (isInterior && !explicitOwner && doors.length > 1) {
        const remote = unownedInteriorEntrances(
          doors.map((door) => {
            const id = doorIndices.get(door)!;
            const { height } = endpointBinding(
              `door-outside/${id}`,
              door.sector_out,
              door.layer_out,
              door.point_out,
            );
            return { door: id, point: [door.point_in[0], door.point_in[1] + height], height };
          }),
          [...locals].flatMap(([index, owners]) =>
            owners.length === 1 && owners[0]!.asset === owner.asset
              ? [proto.sight_obstacles[index]!]
              : [],
          ),
        );
        if (remote.length) {
          unresolved.push({
            kind: "interior-entrance-ownership",
            building: index,
            asset: owner.asset,
            entrances: remote,
            reason:
              "Every entrance needs local owning geometry; partition this shared room into explicit asset-owned pieces",
          });
          continue;
        }
      }
      if (!explicitOwner && !stateOwner && !choose(ranked))
        packet(owner.asset).issues.push(
          "Door ownership resolved from geometry above its landing; review the physical doorway before publication",
        );
      const endpoints = doors.map((door, i) => {
        const outside = endpointBinding(
          `door-outside/${doorIndices.get(door)!}`,
          door.sector_out,
          door.layer_out,
          door.point_out,
        );
        const inside = isInterior
          ? undefined
          : endpointBinding(
              `door-inside/${doorIndices.get(door)!}`,
              door.sector_in,
              door.layer_in,
              door.point_in,
            );
        const elevation = outside.height;
        const local = (point: Point) =>
          localize(owner.part, [point[0], point[1] + elevation, elevation]);
        return {
          id: `door-${i}`,
          node: owner.node,
          polygon: door.door_sector.points.map(local),
          outside: local(door.point_out),
          inside: isInterior
            ? local(door.point_in)
            : localize(owner.part, [
                door.point_in[0],
                door.point_in[1] + inside!.height,
                inside!.height,
              ]),
          ...(outside.anchor ? { outsideAnchor: localize(owner.part, outside.anchor) } : {}),
          ...(inside?.anchor ? { insideAnchor: localize(owner.part, inside.anchor) } : {}),
          middle: local(door.point_mid),
          type: door.door_type,
          active: door.active,
          ...(!isInterior &&
          door.door_type === 0 &&
          door.active &&
          !door.door_sector.points.length &&
          !door.locked_pc &&
          !door.unlockable &&
          !door.locked_npc_villain &&
          !door.locked_npc_civilian &&
          !door.locked_pc_after_patch &&
          !door.unlockable_after_patch &&
          !door.locked_npc_villain_after_patch &&
          !door.locked_npc_civilian_after_patch &&
          !proto.patches.some((patch) => patch.door_indices.includes(doorIndices.get(door)!))
            ? { allowContinuous: true }
            : {}),
          locks: {
            player: door.locked_pc,
            unlockable: door.unlockable,
            villains: door.locked_npc_villain,
            civilians: door.locked_npc_civilian,
          },
          afterTransition: {
            player: door.locked_pc_after_patch,
            unlockable: door.unlockable_after_patch,
            villains: door.locked_npc_villain_after_patch,
            civilians: door.locked_npc_civilian_after_patch,
          },
        };
      });
      const connectionId = `${isInterior ? "interior" : "passage"}-${packet(owner.asset).connections.length}`;
      packet(owner.asset).connections.push({
        id: connectionId,
        node: owner.node,
        kind: connection.kind,
        endpoints,
        ...(connection.authoring
          ? {
              interiorJoins: connection.authoring.joins.map((join) => {
                const origin = localize(owner.part, [0, 0, 0]);
                const direction = localize(owner.part, [...join.direction, 0]);
                return {
                  point: localize(owner.part, join.point),
                  direction: [direction[0] - origin[0], direction[1] - origin[1]] as Point,
                };
              }),
            }
          : {}),
      });
      if (stateOwner) {
        doorStateOwnershipRecovery.push({
          building: index,
          doors: doors.map((door) => doorIndices.get(door)!),
          asset: owner.asset,
          connection: connectionId,
        });
        packet(owner.asset).issues.push(
          "Door ownership follows linked state geometry; review physical asset grouping before publication",
        );
      }
      doors.forEach((door, i) => {
        recoveredDoors.set(doorIndices.get(door)!, {
          asset: owner.asset,
          id: `${connectionId}/${endpoints[i]!.id}`,
          node: owner.node,
          part: owner.part,
        });
      });
      recovered++;
    } catch (error) {
      unresolved.push({
        kind: "building-endpoint",
        building: index,
        sourceDoor,
        reason: String(error),
      });
    }
  }
  if (recovered === recoveryGroups.length) recoveredBuildings++;
}
const doorTransitionRecovery: { patch: number; asset: string; transition: string }[] = [];
for (const [index, source] of proto.patches.entries()) {
  if (!source.door_indices.length) continue;
  try {
    const transitions = movementTransitionRecovery.filter((entry) => entry.patch === index);
    const doorOwners = source.door_indices.map((door) => {
      const owner = recoveredDoors.get(door);
      if (!owner) throw new Error(`Missing door ownership ${door}`);
      return owner;
    });
    const owner = doorOwners[0]!;
    if (doorOwners.some((entry) => entry.asset !== owner.asset))
      throw new Error("Linked doors belong to different assets");
    if (!source.door_triggered && !source.triggers_door)
      throw new Error("Door binding has neither trigger nor rights-swap semantics");
    if (transitions.length > 1)
      throw new Error("Door binding needs one recovered asset-local transition");
    const recovered = transitions[0];
    if (recovered && recovered.asset !== owner.asset)
      throw new Error("Door and its transition belong to different assets");
    if (
      !recovered &&
      movementStateInventory.some((area) =>
        area.transitions.some((change) => change.patches.includes(index)),
      )
    )
      throw new Error("Door transition has unrecovered movement changes");
    const sightRef = (index: number) => {
      const candidates = locals.get(index) ?? [];
      if (candidates.length !== 1)
        throw new Error(`Door transition needs one sight owner for obstacle ${index}`);
      const sight = candidates[0]!;
      if (sight.asset !== owner.asset)
        throw new Error(`Door and sight obstacle ${index} belong to different assets`);
      return sight.collisionId ?? sight.node;
    };
    const initialSight = source.old_sight_obstacles.map(sightRef);
    const appliedSight = source.new_sight_obstacles.map(sightRef);
    const p = packet(owner.asset);
    const waypoint = endpointBinding(
      `patch-waypoint/${index}`,
      source.sector,
      source.layer,
      source.waypoint,
    );
    const transition = recovered
      ? p.movementTransitions!.find((entry) => entry.id === recovered.transition)!
      : recoverMovementTransition({
          id: `door-change-${index}`,
          node: owner.node,
          patch: source,
          initial: [],
          applied: [],
          initialSight,
          appliedSight,
          receivers: [],
          groundLayer: source.layer === 0,
          waypointHeight: waypoint.height,
          localize: (point) => localize(owner.part, point),
        });
    transition.doorLinks = {
      mode: source.door_triggered ? "trigger-transition" : "swap-rights",
      ids: doorOwners.map((entry) => entry.id),
    };
    if (waypoint.anchor) transition.waypointAnchor = localize(owner.part, waypoint.anchor);
    if (
      !recovered &&
      (initialSight.length || appliedSight.length) &&
      p.movementBlockers === undefined
    ) {
      const descriptor = descriptors.get(owner.asset)!;
      const controlled = new Set([...initialSight, ...appliedSight]);
      const solids = p.movementSolids ?? [
        ...descriptor.parts
          .filter((part) => part.obstacle_local_game?.solid)
          .map((part) => part.node),
        ...(p.volumes ?? []).filter((volume) => volume.shape.solid).map((volume) => volume.id),
      ];
      p.movementSolids = solids.filter((ref) => !controlled.has(ref));
    }
    if (!recovered) (p.movementTransitions ??= []).push(transition);
    p.issues.push("Door bindings recovered; visual states and effects need separate authoring");
    doorTransitionRecovery.push({
      patch: index,
      asset: owner.asset,
      transition: transition.id,
    });
  } catch (error) {
    unresolved.push({ kind: "door-transition", patch: index, reason: String(error) });
  }
}
// Ground regions belong to the terrain asset. Obstacle-only regions must have
// explicit local owners; unresolved projection links stay in the recovery report.
const groundMaterialOwners = [...descriptors.values()].filter(
  (d) => d.editor_usage === "map-background",
);
if (groundMaterialOwners.length === 1) {
  packet(groundMaterialOwners[0]!.id).environment = {
    forest: proto.misc.forest_level,
    defaultMaterial: proto.misc.default_material,
  };
} else
  unresolved.push({
    kind: "map-environment-owner",
    candidates: groundMaterialOwners.map((d) => d.id),
  });
for (const index of proto.sight_material_indices) {
  const region = proto.material_sectors[index];
  if (!region || groundMaterialOwners.length !== 1) {
    unresolved.push({
      kind: "ground-material-owner",
      index,
      candidates: groundMaterialOwners.map((d) => d.id),
    });
    continue;
  }
  const p = packet(groundMaterialOwners[0]!.id);
  (p.materials ??= []).push({
    id: `ground-material-${p.materials?.length ?? 0}`,
    node: "$root",
    material: region.material,
    ground: true,
    obstacles: [],
    polygon: region.polygon.points.map(([x, y]) => [x, y, 0]),
  });
  recoveredMaterials.add(index);
}
for (const [index, obstacle] of proto.sight_obstacles.entries()) {
  if (!obstacle.material_indices.length) continue;
  if (obstacle.projection_area !== null && recoveredProjectionMaterials.has(index)) continue;
  const owners = locals.get(index) ?? [];
  if (!owners.length || obstacle.projection_area !== null) {
    unresolved.push({
      kind: "obstacle-material-owner",
      obstacle: index,
      reason:
        obstacle.projection_area !== null
          ? "Projection-surface material links still need recovery"
          : "No asset-local owner",
    });
    continue;
  }
  for (const material of obstacle.material_indices) {
    const region = proto.material_sectors[material];
    if (!region) throw new Error(`Missing material region ${material}`);
    for (const owner of owners) {
      const p = packet(owner.asset);
      (p.materials ??= []).push({
        id: `obstacle-material-${p.materials?.length ?? 0}`,
        node: owner.node,
        material: region.material,
        ground: false,
        obstacles: [owner.collisionId ?? owner.node],
        polygon: region.polygon.points.map(([x, y]) => localize(owner.part, [x, y, 0])),
      });
    }
    recoveredMaterials.add(material);
  }
}
const authoredSounds = recoverAuthoredSounds(document, descriptors, proto.sound_sources);
const authoredSoundSources = new Set(authoredSounds.flatMap((asset) => asset.sourceIndices));
const declaredSounds = declaredSoundOwners(
  ownership?.sound_sources ?? [],
  proto.sound_sources,
  (asset, node) =>
    [...locals.values()].flat().filter((owner) => owner.asset === asset && owner.node === node),
  authoredSoundSources,
);
for (const asset of authoredSounds) packet(asset.asset).sounds = asset.sounds;
let recoveredSounds = authoredSoundSources.size;
for (const [index, sound] of proto.sound_sources.entries()) {
  if (authoredSoundSources.has(index)) continue;
  if (sound.global && groundMaterialOwners.length === 1) {
    const p = packet(groundMaterialOwners[0]!.id);
    (p.sounds ??= []).push(recoverSoundSource(sound, `ambient-sound-${index}`, "$root", (p) => p));
    recoveredSounds++;
    continue;
  }
  const owners = [...locals.values()].flat().filter((owner) => {
    const shape = owner.sourceShape ?? transformedObstacle(document, owner.part);
    return (
      sound.polyline &&
      containsSoundPolyline(
        sound.polyline,
        shape.points.map((p) => [p.x, p.y]),
      )
    );
  });
  const soundOwner = declaredSounds.get(index) ?? uniqueSoundOwner(owners);
  if (!soundOwner) {
    unresolved.push({
      kind: "sound-owner",
      source: index,
      sample: sound.id,
      candidates: owners.map(({ asset, node }) => ({ asset, node })),
      reason: "Local emitter needs explicit asset ownership; no terrain fallback",
    });
    continue;
  }
  const owner = soundOwner;
  const p = packet(owner.asset);
  (p.sounds ??= []).push(
    recoverSoundSource(sound, `ambient-sound-${index}`, owner.node, (point) =>
      localize(owner.part, point),
    ),
  );
  if (!declaredSounds.has(index))
    p.issues.push(
      "Review environmental sound ownership inferred from unique geometric containment",
    );
  recoveredSounds++;
}
let recoveredLights = 0;
const declaredLights = declaredLightOwners(
  ownership?.light_sources ?? [],
  proto.light_sectors,
  (asset, node) =>
    document.objects.filter(
      (part) =>
        descriptors.get(asset)?.parts.some((p) => p.node === node) &&
        part.node === `asset:${asset}:${node}`,
    ),
);
const lightRecovery: { source: number; asset: string; ids: string[] }[] = [];
for (const [index, light] of proto.light_sectors.entries()) {
  try {
    const { region, footprints: contours } = recoverLightField(
      light,
      `light-${index}`,
      proto.sight_obstacles,
      proto.motion_data.layers[light.layer] ?? [],
      [...sourceMotionAreas]
        .filter(([, area]) => area.layer === light.layer)
        .map(([sector]) => sector),
    );
    const regions = [region];
    const allOwners = [...locals.values()].flat();
    const owners = [...new Set(allOwners.map((owner) => owner.asset))].flatMap((asset) => {
      const parts = allOwners.filter((owner) => owner.asset === asset);
      const footprints = parts.map((owner) =>
        (owner.sourceShape ?? transformedObstacle(document, owner.part)).points.map((p): Point => [
          p.x,
          p.y,
        ]),
      );
      return contours.every((contour) => containsLightPolygon(contour, footprints))
        ? [parts[0]!]
        : [];
    });
    const selected = declaredLights.get(index) ?? (owners.length === 1 ? owners[0] : undefined);
    if (!selected) {
      unresolved.push({
        kind: "light-owner",
        source: index,
        candidates: owners.map(({ asset, node }) => ({ asset, node })),
        reason: "Light region needs explicit asset ownership; no terrain fallback",
      });
      continue;
    }
    const owner = selected,
      p = packet(owner.asset);
    (p.lights ??= []).push(
      ...regions.map((region) => ({
        ...region,
        node: owner.node,
        polygon: region.polygon.map((point) => localize(owner.part, point)),
        ...(region.receivers
          ? { receivers: region.receivers.map((point) => localize(owner.part, point)) }
          : {}),
        ...(region.receiverSegments
          ? {
              receiverSegments: region.receiverSegments.map(([a, b]): [Vec3, Vec3] => [
                localize(owner.part, a),
                localize(owner.part, b),
              ]),
            }
          : {}),
      })),
    );
    lightRecovery.push({
      source: index,
      asset: owner.asset,
      ids: regions.map((region) => region.id),
    });
    if (!declaredLights.has(index))
      p.issues.push("Review light-region ownership inferred from unique geometric containment");
    recoveredLights++;
  } catch (error) {
    unresolved.push({ kind: "light-geometry", source: index, error: String(error) });
  }
}
let recoveredJumps = 0;
type JumpOwner = { asset: string; node: string; part?: Level3DObject };
const jumpPoint = (owner: JumpOwner, p: Vec3) => (owner.part ? localize(owner.part, p) : p);
const jumpReceivingFootprints = (asset: string, zone: ProtoLevel["jump_zones"][number]) => {
  const footprints = proto.sight_obstacles.flatMap((obstacle, index) => {
    if (
      !Array.isArray(obstacle.projection_area) ||
      obstacle.projection_area[0] !== zone.sector ||
      obstacle.projection_area[1] !== zone.layer
    )
      return [];
    return (locals.get(index) ?? [])
      .filter((owner) => owner.asset === asset)
      .map((owner) =>
        (owner.sourceShape ?? transformedObstacle(document, owner.part)).points.map((p): Point => [
          p.x,
          p.y - p.z_top,
        ]),
      );
  });
  // Ground-only landing zones have no obstacle-backed projection footprint.
  return footprints.length ? footprints : undefined;
};
for (const [index, pair] of proto.jump_line_pairs.entries()) {
  try {
    const sideCandidates = [pair.line1, pair.line2].map((line, side): JumpOwner[] => {
      const home = proto.jump_zones[(side === 0 ? pair.line2 : pair.line1).jump_zone_index];
      if (!home) throw new Error("Jump pair references a missing receiving zone");
      const supports = proto.sight_obstacles.flatMap((o, obstacleIndex) =>
        Array.isArray(o.projection_area) &&
        o.projection_area[0] === home.sector &&
        o.projection_area[1] === home.layer
          ? (locals.get(obstacleIndex) ?? [])
          : [],
      );
      const sideOwners = jumpEdgeOwners(
        line,
        supports,
        (owner) => owner.sourceShape ?? transformedObstacle(document, owner.part),
      );
      if (!sideOwners.length && home.layer !== 0)
        throw new Error(`Jump side ${side} has no owned elevated receiving surface`);
      return sideOwners;
    });
    if (sideCandidates.every((side) => !side.length) && groundMaterialOwners.length === 1) {
      const terrain = packet(groundMaterialOwners[0]!.id);
      if (
        terrainOwnsJump(
          pair,
          proto.jump_zones,
          groundAreaSectors,
          terrain.surfaces,
          transferredGroundExclusions,
        )
      ) {
        const owner = { asset: terrain.asset, node: "$root" };
        sideCandidates[0]!.push(owner);
        sideCandidates[1]!.push(owner);
      }
    }
    const candidates = sideCandidates.flat();
    const assets = new Set(candidates.map((o) => o.asset));
    if (
      assets.size === 2 &&
      sideCandidates.every((c) => new Set(c.map((o) => o.asset)).size === 1)
    ) {
      const recovered = ([0, 1] as const).map((side) => {
        const owner =
          [...locals.values()].flat().find((o) => o.asset === sideCandidates[side]![0]!.asset) ??
          sideCandidates[side]![0]!;
        return {
          owner,
          ...recoverJumpSegment(
            side,
            proto,
            index,
            owner.node,
            (point) => jumpPoint(owner, point),
            (zone, point) => heightAt(zone.sector, zone.layer, point),
            (zone) => jumpReceivingFootprints(owner.asset, zone),
          ),
        };
      });
      for (const { owner, zone } of recovered) {
        const previous = packet(owner.asset).jumpZones?.find((z) => z.id === zone.id);
        if (previous && JSON.stringify(previous) !== JSON.stringify(zone))
          throw new Error(`Shared jump zone ${zone.id} needs consistent owner and anchor`);
      }
      for (const { owner, zone, segment } of recovered) {
        const p = packet(owner.asset);
        if (!p.jumpZones?.some((z) => z.id === zone.id)) (p.jumpZones ??= []).push(zone);
        (p.jumpSegments ??= []).push(segment);
        p.issues.push("Review cross-asset jump sockets and landing ownership before publication");
      }
      recoveredJumps++;
      continue;
    }
    if (assets.size !== 1) {
      unresolved.push({
        kind: "jump-owner",
        source: index,
        candidates: [...assets],
        reason:
          "Each jump side needs one explicit asset owner; ambiguous ownership must be authored",
      });
      continue;
    }
    const owner =
        [...locals.values()].flat().find((o) => o.asset === candidates[0]!.asset) ?? candidates[0]!,
      p = packet(owner.asset);
    const recovered = recoverJumpGeometry(
      proto,
      index,
      owner.node,
      (point) => jumpPoint(owner, point),
      (zone, point) => heightAt(zone.sector, zone.layer, point),
      (zone) => jumpReceivingFootprints(owner.asset, zone),
    );
    for (const zone of recovered.zones) {
      const previous = p.jumpZones?.find((z) => z.id === zone.id);
      if (previous && JSON.stringify(previous) !== JSON.stringify(zone))
        throw new Error(`Shared jump zone ${zone.id} needs consistent owner and anchor`);
    }
    for (const zone of recovered.zones)
      if (!p.jumpZones?.some((z) => z.id === zone.id)) (p.jumpZones ??= []).push(zone);
    (p.jumpPairs ??= []).push(recovered.pair);
    p.issues.push(
      "Review jump ownership inferred from projection surfaces, landing anchors and click-region height",
    );
    recoveredJumps++;
  } catch (error) {
    unresolved.push({ kind: "jump-geometry", source: index, error: String(error) });
  }
}
let projectionRecovery: ReturnType<typeof recoverReviewedProjections> = [];
if (values["projection-definitions"]) {
  const definitions: ReviewedProjections = JSON.parse(
    await fs.readFile(values["projection-definitions"], "utf8"),
  );
  projectionRecovery = recoverReviewedProjections(
    document,
    descriptors,
    proto,
    createHash("sha256").update(sourceBytes).digest("hex"),
    definitions,
    packets,
  );
}
let maskRecovery: Awaited<ReturnType<typeof recoverReviewedMasks>> = [];
const maskTransitionRecovery: { patch: number; asset: string; transition: string }[] = [];
if (values["mask-definitions"]) {
  const definitions: { source_sha256: string; recipes: ReviewedMaskRecipe[] } = JSON.parse(
    await fs.readFile(values["mask-definitions"], "utf8"),
  );
  if (createHash("sha256").update(sourceBytes).digest("hex") !== definitions.source_sha256)
    throw new Error("Reviewed mask source changed");
  const maskOwners = new Map<number, { asset: string; id: string; node: string }>();
  for (const recipe of definitions.recipes)
    for (const entry of recipe.entries) {
      if (maskOwners.has(entry.source)) throw new Error("Duplicate reviewed mask index");
      maskOwners.set(entry.source, { asset: recipe.asset, id: entry.id, node: entry.node });
    }
  const resolveMask = maskReferenceResolver(proto.masks);
  for (const [index, source] of proto.patches.entries()) {
    const masks = [...resolveMask(source.old_masks), ...resolveMask(source.new_masks)];
    const owner = masks.map((mask) => maskOwners.get(mask)).find((entry) => entry !== undefined);
    if (
      !owner ||
      [...movementTransitionRecovery, ...doorTransitionRecovery].some((t) => t.patch === index)
    )
      continue;
    if (
      source.door_indices.length ||
      movementStateInventory.some((area) =>
        area.transitions.some((change) => change.patches.includes(index)),
      )
    )
      throw new Error(`Mask transition ${index} has unrecovered movement or door behavior`);
    recoverMaskStateLinks(proto.masks, source, owner.asset, maskOwners);
    const parts = document.objects.filter(
      (part) => part.node === `asset:${owner.asset}:${owner.node}`,
    );
    if (parts.length !== 1)
      throw new Error(`Mask transition ${index} needs one pinned asset frame`);
    const part = parts[0]!;
    const sightRef = (ref: number) => {
      const matches = locals.get(ref) ?? [];
      if (matches.length !== 1 || matches[0]!.asset !== owner.asset)
        throw new Error(`Mask transition ${index} needs local sight ownership for ${ref}`);
      return matches[0]!.collisionId ?? matches[0]!.node;
    };
    const initialSight = source.old_sight_obstacles.map(sightRef);
    const appliedSight = source.new_sight_obstacles.map(sightRef);
    const waypoint = endpointBinding(
      `patch-waypoint/${index}`,
      source.sector,
      source.layer,
      source.waypoint,
    );
    const transition = recoverMovementTransition({
      id: `mask-change-${index}`,
      node: owner.node,
      patch: source,
      initial: [],
      applied: [],
      initialSight,
      appliedSight,
      receivers: [],
      groundLayer: source.layer === 0,
      waypointHeight: waypoint.height,
      localize: (point) => localize(part, point),
    });
    if (waypoint.anchor) transition.waypointAnchor = localize(part, waypoint.anchor);
    const p = packet(owner.asset);
    if ((initialSight.length || appliedSight.length) && p.movementBlockers === undefined) {
      const descriptor = descriptors.get(owner.asset)!;
      const controlled = new Set([...initialSight, ...appliedSight]);
      const solids = p.movementSolids ?? [
        ...descriptor.parts
          .filter((part) => part.obstacle_local_game?.solid)
          .map((part) => part.node),
        ...(p.volumes ?? []).filter((volume) => volume.shape.solid).map((volume) => volume.id),
      ];
      p.movementSolids = solids.filter((ref) => !controlled.has(ref));
    }
    (p.movementTransitions ??= []).push(transition);
    p.issues.push("Mask/sight state recovered; visual states and effects need separate authoring");
    maskTransitionRecovery.push({ patch: index, asset: owner.asset, transition: transition.id });
  }
  maskRecovery = await recoverReviewedMasks(values.library, document, proto, definitions.recipes, [
    ...movementTransitionRecovery,
    ...doorTransitionRecovery,
    ...maskTransitionRecovery,
  ]);
  for (const recovered of maskRecovery) {
    const p = packet(recovered.asset);
    const state = recovered.state;
    if (state) {
      const targets = p.movementTransitions?.filter((t) => t.id === state.transition) ?? [];
      if (targets.length !== 1)
        throw new Error(`Recovered mask ${recovered.source} needs one local transition`);
      const key = state.phase === "initial" ? "initialMasks" : "appliedMasks";
      (targets[0]![key] ??= []).push(recovered.definition.id);
    }
    (p.masks ??= []).push(recovered.definition);
  }
}
const appearanceRecovery = recoverAppearanceBindings(
  inputDocument,
  proto.patches.length,
  [...movementTransitionRecovery, ...doorTransitionRecovery, ...maskTransitionRecovery],
  new Map([...packets].map(([id, p]) => [id, p.movementTransitions ?? []])),
);
for (const binding of appearanceRecovery.bindings) {
  const transition = packet(binding.asset).movementTransitions!.find(
    (t) => t.id === binding.transition,
  )!;
  transition.appearances = [...new Set([...(transition.appearances ?? []), binding.appearance])];
}
for (const entry of appearanceRecovery.unresolved)
  unresolved.push({ kind: "appearance-binding", ...entry });
// A scene frame or preview box is not proof that its physical volume migrated.
// Inventory all source records, including ones only referenced by masks.
const unownedSightObstacles = proto.sight_obstacles.flatMap((shape, index) => {
  const physicalOwners = (locals.get(index) ?? []).filter((owner) => {
    if (owner.collisionId)
      return packets.get(owner.asset)?.volumes?.some((volume) => volume.id === owner.collisionId);
    const part = descriptors.get(owner.asset)?.parts.find((part) => part.node === owner.node);
    return part?.obstacle_local_game && part.mission_profile === undefined;
  });
  if (physicalOwners.length) return [];
  return [
    {
      obstacle: index,
      solid: shape.solid,
      opaque: shape.opaque,
      mouse: shape.mouse,
      masks: proto.masks.flatMap((mask, maskIndex) =>
        mask.obstacle_indices.includes(index) ? [maskIndex] : [],
      ),
      patches: proto.patches.flatMap((patch, patchIndex) =>
        [...patch.old_sight_obstacles, ...patch.new_sight_obstacles].includes(index)
          ? [patchIndex]
          : [],
      ),
    },
  ];
});
for (const entry of unownedSightObstacles) unresolved.push({ kind: "sight-owner", ...entry });
const pending = {
  appearanceBindings: appearanceRecovery.unresolved.length,
  sightObstacleOwners: unownedSightObstacles.length,
  doorTransitionBindings:
    proto.patches.filter((patch) => patch.door_indices.length > 0).length -
    doorTransitionRecovery.length,
  movementTransitions:
    movementStateInventory.reduce((sum, area) => sum + area.transitions.length, 0) -
    movementTransitionRecovery.length,
  buildingEntries: proto.buildings.length - recoveredBuildings,
  maskRecords: proto.masks.length - maskRecovery.length,
  patches: proto.patches.length,
  jumpPairs: proto.jump_line_pairs.length - recoveredJumps,
  materialRegions: proto.material_sectors.length - recoveredMaterials.size,
  shadowRegions: proto.light_sectors.length - recoveredLights,
  soundSources: proto.sound_sources.length - recoveredSounds,
};
if (values["require-movement-coverage"] && pending.movementTransitions)
  throw new Error(
    `Incomplete movement recovery: ${pending.movementTransitions} source transitions lack asset definitions; check scene ownership and authoring recipes`,
  );
for (const [order, owners] of locals)
  for (const owner of owners) {
    const p = packet(owner.asset);
    const id = owner.collisionId ?? owner.node;
    const orders = (p.sightOrder ??= {});
    if (orders[id] !== undefined && orders[id] !== order)
      throw new Error(`Ambiguous physical query order: ${owner.asset}/${id}`);
    orders[id] = order;
  }
await fs.mkdir(values.out, { recursive: true });
const definitionValidation: { asset: string; valid: boolean; error?: string }[] = [];
for (const [asset, p] of packets) {
  try {
    const descriptor = descriptors.get(asset);
    if (!descriptor) throw new Error(`Missing pinned descriptor ${asset}`);
    p.gameplayCandidate = recoveredGameplayDefinition(p, descriptor);
    definitionValidation.push({ asset, valid: true });
  } catch (error) {
    definitionValidation.push({ asset, valid: false, error: String(error) });
  }
  p.issues = [...new Set(p.issues)];
  await fs.writeFile(
    path.join(values.out, `${asset}.gameplay-authoring.json`),
    JSON.stringify(p, null, 2) + "\n",
  );
}
// Probe the assembled scene using only the candidate asset definitions. Keep
// this diagnostic separate from publication and from recovery coverage.
const candidates = new Map<string, GameplayAssetDescriptor>(descriptors);
for (const [id, descriptor] of candidates) {
  const gameplay = packets.get(id)?.gameplayCandidate;
  if (gameplay) candidates.set(id, { ...descriptor, gameplay });
}
const diagnostics = diagnoseGameplayCandidates(document, candidates, {
  omittedMovementTransitions: pending.movementTransitions,
});
const report = {
  status: "incomplete-authoring-recovery",
  requiredMovementCoverage: values["require-movement-coverage"],
  assets: packets.size,
  files: [...packets.keys()].sort().map((asset) => `${asset}.gameplay-authoring.json`),
  surfaces: [...packets.values()].reduce((sum, p) => sum + p.surfaces.length, 0),
  connections: [...packets.values()].reduce((sum, p) => sum + p.connections.length, 0),
  movementBlockers: [...packets.values()].reduce(
    (sum, p) => sum + (p.movementBlockers?.length ?? 0),
    0,
  ),
  definitionValidation,
  navigationJoinRecovery,
  lightRecovery,
  authoredSoundRecovery: authoredSounds.map(({ asset, sourceIndices }) => ({
    asset,
    sources: sourceIndices,
  })),
  maskRecovery: maskRecovery.map(({ asset, source, definition, state }) => ({
    asset,
    source,
    id: definition.id,
    ...(state ? { state } : {}),
  })),
  movementStateInventory,
  projectionRecovery,
  movementTransitionRecovery,
  doorTransitionRecovery,
  maskTransitionRecovery,
  appearanceRecovery,
  doorStateOwnershipRecovery,
  declaredEndpointBindings: [...endpointBindings.values()],
  declaredInteriorRecovery: [...interiorSources].map(([building, pieces]) => ({
    building,
    pieces: pieces.map((piece) => ({
      asset: piece.owner,
      node: piece.node,
      doors: piece.doors,
      sockets: piece.joins.length,
    })),
  })),
  declaredDoorOwnershipRecovery: [...declaredDoors].map(([door, owner]) => ({
    door,
    asset: owner.asset,
    node: owner.node,
  })),
  candidateCompilation: diagnostics.compilation,
  staticGeometryDiagnostic: diagnostics.staticGeometry,
  coverage,
  unownedSightObstacles,
  unresolved,
  pending,
};
await fs.writeFile(
  path.join(values.out, "recovery-report.json"),
  JSON.stringify(report, null, 2) + "\n",
);
console.log(
  JSON.stringify({
    assets: report.assets,
    surfaces: report.surfaces,
    connections: report.connections,
    unresolved: unresolved.length,
    pending,
  }),
);
