import { validateMission, type Level3D, type Point } from "@rle/shared";
import type { CompiledAssetGeometry } from "../../shared/src/asset-gameplay.ts";
import { heightPlane, planeHeight } from "../../shared/src/gameplay-plane.ts";

function contains(polygon: Point[], point: Point): boolean {
  let inside = false;
  for (let i = 0, j = polygon.length - 1; i < polygon.length; j = i++) {
    const a = polygon[i]!,
      b = polygon[j]!;
    if (
      a[1] > point[1] !== b[1] > point[1] &&
      point[0] < ((b[0] - a[0]) * (point[1] - a[1])) / (b[1] - a[1]) + a[0]
    )
      inside = !inside;
  }
  return inside;
}

/** Mission markers use game-world XYZ; exported positions use projected map pixels. */
export function compileMission(
  document: Level3D,
  bounds: [number, number, number, number],
  geometry: CompiledAssetGeometry | undefined,
  bestEffort = false,
  volumes: { footprint: Point[]; motion_blocking: boolean }[] = [],
) {
  const spawn_points: {
    position: Point;
    direction: number;
    profile?: number;
    sector: number;
    layer: number;
    projection_area: number;
  }[] = [];
  const soldiers: {
    position: Point;
    direction: number;
    profile: string;
    allegiance: number;
    sector: number;
    layer: number;
    projection_area: number;
  }[] = [];
  const warnings: string[] = [];
  if (!document.mission) return { spawn_points, soldiers, warnings };
  validateMission(document.mission);
  warnings.push(
    ...(document.mission.importWarnings ?? []).map((warning) => `Imported mission: ${warning}`),
  );
  const areas: {
    sector: number;
    layer: number;
    area: CompiledAssetGeometry["motion_data"]["layers"][number][number];
  }[] = [];
  let sector = 0;
  geometry?.motion_data.layers.forEach((layer, index) =>
    layer.forEach((area) => {
      areas.push({ sector, layer: index, area });
      sector += 1 + area.obstacles.length;
    }),
  );
  const bind = (actor: { id: string; position: [number, number, number] }) => {
    const [x, y, z] = actor.position;
    const position: Point = [Math.round(x - bounds[0]), Math.round(y - z - bounds[1])];
    if (
      position.some((v) => v < 0 || v > 32767) ||
      position[0] >= bounds[2] ||
      position[1] >= bounds[3]
    )
      throw new Error(`Mission ${actor.id}: position is outside the export frame`);
    if (!geometry) {
      if (
        Math.abs(z) > 1 ||
        position[0] >= bounds[2] - 1 ||
        position[1] >= bounds[3] - 1 ||
        volumes.some((volume) => volume.motion_blocking && contains(volume.footprint, position))
      )
        throw new Error(`Mission ${actor.id}: position is not on the sandbox's walkable ground`);
      return { position, sector: 0, layer: 0, projection_area: 65535 };
    }
    const matches = areas.flatMap(({ sector, layer, area }) => {
      if (
        area.is_lift ||
        !contains(area.polygon.points, position) ||
        area.obstacles.some(
          (o) =>
            (o.state_id & 0x55555555) >>> 0 === o.state_id && contains(o.polygon.points, position),
        )
      )
        return [];
      const heights = geometry.sight_obstacles.flatMap((obstacle, index) => {
        const receiver = obstacle.projection_area;
        if (
          !Array.isArray(receiver) ||
          receiver[0] !== sector ||
          receiver[1] !== layer ||
          !contains(
            obstacle.points.map((p) => [p.x, p.y - p.z_top]),
            position,
          )
        )
          return [];
        const points =
          obstacle.projection_plane ??
          obstacle.points.map((p) => [p.x, p.y, p.z_top] as [number, number, number]);
        return [
          {
            height: planeHeight(heightPlane(points.map(([a, b, c]) => [a, b - c, c])), position),
            index,
          },
        ];
      });
      const receiver = heights.find(({ height }) => Math.abs(height - z) <= 1);
      if (receiver) {
        if (receiver.index >= 65535)
          throw new Error(`Mission ${actor.id}: receiving surface exceeds the native index range`);
        return [{ sector, layer, projection_area: receiver.index }];
      }
      return !heights.length && Math.abs(z) <= 1 ? [{ sector, layer, projection_area: 65535 }] : [];
    });
    if (matches.length !== 1)
      throw new Error(
        `Mission ${actor.id}: position does not identify one walkable surface at height ${z}`,
      );
    return { position, ...matches[0]! };
  };
  for (const actor of [
    ...document.mission.spawnPoints.map((point) => ({ kind: "pc" as const, point })),
    ...document.mission.soldiers.map((point) => ({ kind: "soldier" as const, point })),
  ]) {
    const [x, y, z] = actor.point.position;
    const projected: Point = [Math.round(x - bounds[0]), Math.round(y - z - bounds[1])];
    if (
      projected[0] < 0 ||
      projected[1] < 0 ||
      projected[0] >= bounds[2] ||
      projected[1] >= bounds[3]
    ) {
      warnings.push(
        `Mission ${actor.point.id}: outside the export frame; omitted this mission marker and retained it in the editor document.`,
      );
      continue;
    }
    try {
      const placement = bind(actor.point);
      if (actor.kind === "pc")
        spawn_points.push({
          ...placement,
          direction: actor.point.direction,
          ...(actor.point.profile === undefined ? {} : { profile: actor.point.profile }),
        });
      else
        soldiers.push({
          ...placement,
          direction: actor.point.direction,
          profile: actor.point.profile,
          allegiance: actor.point.allegiance,
        });
    } catch (error) {
      if (!bestEffort) throw error;
      warnings.push(
        `${error instanceof Error ? error.message : String(error)}; omitted this mission marker.`,
      );
    }
  }
  return { spawn_points, soldiers, warnings };
}

/** Source-only staging boundary. Runtime decoding and gameplay binding are still required. */
export interface NativeMissionSourceRequest {
  version: 1;
  mission: string;
  map: string;
  mission_data: { path: string; sha256: string };
  level_data: { path: string; sha256: string };
  script: { path: string; sha256: string };
  runtime_resources: { path: string; sha256: string }[];
}

export interface NativeSourceIdentity {
  kind: "map-patch" | "mission-patch" | "map-animation";
  index: number;
  /** Runtime patch index includes every map patch, including invisible rows. */
  patch_slot?: number;
}

/** Freeze exact source bytes; never rebuild script identities from editor preview markers. */
export async function prepareNativeMissionSource(
  request: NativeMissionSourceRequest,
  read: (path: string) => Promise<Uint8Array>,
) {
  const spec = structuredClone(request);
  if (
    spec.version !== 1 ||
    !/^[A-Za-z0-9_-]+$/.test(spec.mission) ||
    !/^[A-Za-z0-9_-]+$/.test(spec.map) ||
    spec.mission_data.path !== `Data/Levels/${spec.mission}.rhm.json` ||
    spec.level_data.path !== `Data/Levels/${spec.map}.rhp.json` ||
    spec.script.path !== `Data/Levels/${spec.mission}.scb`
  )
    throw new Error("Invalid native mission source identity or script path");
  const files = new Map<string, Uint8Array>();
  for (const resource of [
    spec.mission_data,
    spec.level_data,
    spec.script,
    ...spec.runtime_resources,
  ]) {
    if (
      !resource.path.startsWith("Data/") ||
      /[\\\0]/.test(resource.path) ||
      resource.path.split("/").some((part) => !part || part === "." || part === "..") ||
      !/^[a-f0-9]{64}$/.test(resource.sha256) ||
      files.has(resource.path)
    )
      throw new Error(`Invalid or duplicate native source resource: ${resource.path}`);
    const bytes = new Uint8Array(await read(resource.path));
    const hash = Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)))
      .map((byte) => byte.toString(16).padStart(2, "0"))
      .join("");
    if (hash !== resource.sha256)
      throw new Error(`Native source resource hash differs: ${resource.path}`);
    if (!bytes.length) throw new Error(`Empty native source resource: ${resource.path}`);
    files.set(resource.path, bytes);
  }
  const parse = (path: string) =>
    JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(files.get(path)!));
  const mission = parse(spec.mission_data.path),
    level = parse(spec.level_data.path);
  if (mission.header?.map_filename !== spec.map || mission.format !== level.format)
    throw new Error("Native mission map or source format differs");
  // Only this construction layout has a qualified patch-index translation.
  if (
    JSON.stringify(level.element_chunk_order) !== JSON.stringify(["Animation", "Patch"]) ||
    JSON.stringify(mission.element_chunk_order) !==
      JSON.stringify(["Patch", "Element", "Bonus", "Scroll", "Mobile"]) ||
    JSON.stringify(mission.element_group_order) !==
      JSON.stringify(["Animal", "BeamMe", "Civilian", "PcToRescue", "Soldier", "Target"])
  )
    throw new Error("Unsupported native source construction order");
  const requiredMissionArrays = [
    "mission_patches",
    "beam_mes",
    "soldiers",
    "civilians",
    "targets",
    "bonuses",
    "pcs_to_rescue",
    "scrolls",
    "building_tenants",
    "hiking_paths",
    "mobile_elements",
  ];
  for (const key of requiredMissionArrays)
    if (!Array.isArray(mission[key])) throw new Error(`Missing native mission source rows: ${key}`);
  for (const key of ["patches", "animations", "masks", "sight_obstacles", "buildings", "lifts"])
    if (!Array.isArray(level[key])) throw new Error(`Missing native level source rows: ${key}`);
  if (!level.motion_data || !mission.script_objects || !mission.tactic_data)
    throw new Error("Missing native navigation or script topology");
  const identities: NativeSourceIdentity[] = [
    ...level.patches.map((_: unknown, index: number) => ({
      kind: "map-patch" as const,
      index,
      patch_slot: index,
    })),
    ...mission.mission_patches.map((_: unknown, index: number) => ({
      kind: "mission-patch" as const,
      index,
      patch_slot: level.patches.length + index,
    })),
    ...level.animations.map((_: unknown, index: number) => ({
      kind: "map-animation" as const,
      index,
    })),
  ];
  const classes = new Set<string>();
  const inspect = (value: unknown): void => {
    if (!value || typeof value !== "object") return;
    for (const [key, child] of Object.entries(value)) {
      if ((key === "script_class" || key === "script") && child != null) {
        if (typeof child !== "string" || !child.length)
          throw new Error("Invalid native script class identity");
        classes.add(child);
      }
      inspect(child);
    }
  };
  inspect(mission);
  return {
    version: 1 as const,
    status: "source-pinned-runtime-validation-required" as const,
    request: spec,
    files,
    mission,
    level,
    identities,
    required_script_classes: [...classes].sort(),
    // A pinned SCB is not proof that classes, profiles and runtime resources resolve.
    runtime_validation_required: true as const,
  };
}
