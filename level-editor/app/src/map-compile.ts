import { transformedObstacle, serializeStoredMap, type Level3D, type Point } from "@rle/shared";
import { encode } from "fast-png";
import { strToU8, zip } from "fflate";
import {
  compileAssetGameplay,
  type CompiledScenerySource,
} from "../../shared/src/compile-asset-gameplay.ts";
import type { ProjectionAssetDescriptor } from "@rle/shared";
import { packageAppearanceRegions, type BakedAppearanceRegion } from "./map-appearance.ts";
import { compileMission } from "./compile-mission.ts";
import type { SceneryResources } from "./scenery-resources.ts";

export type BakeBounds = [number, number, number, number];
export interface CompiledVolume {
  name: string;
  footprint: Point[];
  z_bottom: number;
  z_top: number;
  motion_blocking: boolean;
  opaque: boolean;
}
export interface BakePixels {
  color: Uint8Array;
  depth: Uint16Array;
}

export function validateBakeBounds(bounds: BakeBounds): BakeBounds {
  const [x, y, width, height] = bounds;
  if (!bounds.every(Number.isFinite) || width <= 0 || height <= 0)
    throw new Error("The export frame must have finite coordinates and a positive size.");
  const result: BakeBounds = [
    Math.floor(x),
    Math.floor(y),
    Math.ceil(x + width) - Math.floor(x),
    Math.ceil(y + height) - Math.floor(y),
  ];
  if (result[2] > 16384 || result[3] > 16384 || result[2] * result[3] > 64 * 1024 * 1024)
    throw new Error(
      "The export frame exceeds 16,384 pixels per side or 64 megapixels. Choose a smaller frame.",
    );
  return result;
}

function coordinate(value: number) {
  const rounded = Math.round(value);
  if (!Number.isFinite(value) || rounded < -32768 || rounded > 32767)
    throw new Error(
      "Compiled geometry exceeds the game's signed 16-bit coordinate range. Move it closer to the export frame.",
    );
  return rounded;
}

export function compileVolumes(document: Level3D, bounds: BakeBounds): CompiledVolume[] {
  const hidden = new Set(document.groups.filter((group) => group.hidden).map((group) => group.id));
  return document.objects.flatMap((part) => {
    if (part.hidden || (part.group && hidden.has(part.group)) || !part.obstacle) return [];
    const obstacle = transformedObstacle(document, part);
    if (obstacle.points.length < 3) return [];
    const footprint: Point[] = obstacle.points.map((point) => [
      coordinate(point.x - bounds[0]),
      coordinate(point.y - bounds[1]),
    ]);
    if (
      footprint.every(([x]) => x < 0) ||
      footprint.every(([x]) => x > bounds[2]) ||
      footprint.every(([, y]) => y < 0) ||
      footprint.every(([, y]) => y > bounds[3])
    )
      return [];
    const bottom = Math.min(...obstacle.points.map((point) => point.z_bottom));
    const top = Math.max(...obstacle.points.map((point) => point.z_top));
    if (!Number.isFinite(bottom) || !Number.isFinite(top) || top <= bottom)
      throw new Error(`Object ${part.id} has an invalid collision height.`);
    return [
      {
        name: part.name ?? part.id,
        footprint,
        z_bottom: bottom,
        z_top: top,
        motion_blocking: obstacle.solid && bottom <= 0 && top > 0,
        opaque: obstacle.opaque,
      },
    ];
  });
}

export function compileMap(
  document: Level3D,
  requestedBounds: BakeBounds,
  assets?: ReadonlyMap<string, ProjectionAssetDescriptor>,
  options: { bestEffort?: boolean; onProgress?: (stage: string) => void } = {},
) {
  // TODO: Compile visual/depth state resources and recover remaining asset mask definitions.
  const bounds = validateBakeBounds(requestedBounds);
  const slug =
    document.map
      .toLowerCase()
      .replace(/[^a-z0-9_-]+/g, "-")
      .replace(/^-+|-+$/g, "") || "map";
  // Namespace map and mission names so installation cannot replace a base-game map.
  const name = `editor-${slug}`;
  let scenerySources: CompiledScenerySource[] = [];
  const assetGeometry =
    assets || document.terrain?.cells.length || document.splines?.length
      ? compileAssetGameplay(document, assets ?? new Map(), bounds, {
          ...options,
          onSceneryCompiled: (sources) => {
            scenerySources = sources;
          },
        })
      : undefined;
  const volumes = assetGeometry ? [] : compileVolumes(document, bounds);
  const mission = compileMission(document, bounds, assetGeometry, options.bestEffort, volumes);
  const warnings = assetGeometry
    ? [
        ...(assetGeometry.warnings ?? []),
        "Compiled from asset-local surfaces, sight geometry and doors. Navigation grids and route graphs are constructed by the engine. Only explicitly authored Mission spawns and soldiers are exported.",
        "Visual/depth state resources and mask recovery for existing assets remain incomplete. This export is not a full gameplay-parity certification.",
      ]
    : [
        "This is an unscripted map sandbox. Mission scripts, triggers, and preview population are not exported.",
        "Navigation uses one ground layer. Raised walkways, lifts, jumps, and sloped obstacle heights are not compiled; sight volumes use their full height range.",
        "Scenery without authored obstacles and spline surfaces are visual only. Add obstacle-bearing assets where gameplay collision is required.",
      ];
  const descriptor = {
    title: document.map,
    map_filename: name,
    spawn_points: mission.spawn_points,
    ...(document.mission ? { soldiers: mission.soldiers } : {}),
    walkable_polygon: [
      [0, 0],
      [bounds[2] - 1, 0],
      [bounds[2] - 1, bounds[3] - 1],
      [0, bounds[3] - 1],
    ],
    volumes,
    ...(assetGeometry ? { asset_geometry: assetGeometry } : {}),
  };
  const details = {
    slug: name,
    title: document.map,
    page_url: "",
    author: "",
    map: name,
    uploaded: "",
    description: "Compiled level-editor map sandbox",
    hackable_missions: [name],
  };
  const editorDocument = assets ? serializeStoredMap(document, assets) : structuredClone(document);
  warnings.push(...mission.warnings);
  return { name, bounds, descriptor, details, warnings, editorDocument, scenerySources };
}

export type CompiledMap = ReturnType<typeof compileMap>;

/** PNGs are already compressed; store them in ZIP without a second deflate pass. */
export async function packageCompiledMap(
  compiled: CompiledMap,
  pixels: BakePixels,
  appearance: readonly BakedAppearanceRegion[] = [],
  scenery: SceneryResources = {},
): Promise<Uint8Array> {
  const {
    name,
    bounds: [, , width, height],
  } = compiled;
  if (pixels.color.length !== width * height * 4 || pixels.depth.length !== width * height)
    throw new Error("Rendered map dimensions do not match the export frame.");
  const miniWidth = Math.max(1, Math.round(width / 14)),
    miniHeight = Math.max(1, Math.round(height / 14));
  const mini = new Uint8Array(miniWidth * miniHeight * 3);
  // Area averaging avoids losing narrow features in the minimap.
  for (let y = 0; y < miniHeight; y++)
    for (let x = 0; x < miniWidth; x++) {
      const sums = [0, 0, 0];
      let count = 0;
      for (
        let sy = Math.floor((y * height) / miniHeight);
        sy < Math.floor(((y + 1) * height) / miniHeight);
        sy++
      )
        for (
          let sx = Math.floor((x * width) / miniWidth);
          sx < Math.floor(((x + 1) * width) / miniWidth);
          sx++
        ) {
          for (let c = 0; c < 3; c++) sums[c]! += pixels.color[(sy * width + sx) * 4 + c]!;
          count++;
        }
      for (let c = 0; c < 3; c++) mini[(y * miniWidth + x) * 3 + c] = Math.round(sums[c]! / count);
    }
  const json = (value: unknown) => strToU8(JSON.stringify(value, null, 2) + "\n");
  const prefix = `Data/Levels/Day/${name}`;
  for (const path of Object.keys(scenery))
    if (
      !/^Data\/Animations\/Day\/[a-zA-Z0-9_-]+\.rhs\.d\//.test(path) ||
      path.split("/").some((part) => !part || part === "." || part === "..") ||
      /[\\\0]/.test(path)
    )
      throw new Error(`Invalid scenery resource destination: ${path}`);
  const files = {
    ...scenery,
    ...packageAppearanceRegions(
      prefix,
      width,
      height,
      pixels,
      appearance,
      compiled.descriptor.asset_geometry?.movement_transitions ?? [],
    ),
    "details.json": json(compiled.details),
    [`editor/${name}.rhlos-map.json`]: json(compiled.editorDocument),
    [`Data/Levels/${name}.level.json`]: json(compiled.descriptor),
    [`${prefix}.map.png`]: encode({ width, height, data: pixels.color, channels: 4, depth: 8 }),
    [`${prefix}.min.png`]: encode({
      width: miniWidth,
      height: miniHeight,
      data: mini,
      channels: 3,
      depth: 8,
    }),
    [`${prefix}.occlusion-depth.png`]: encode({
      width,
      height,
      data: pixels.depth,
      channels: 1,
      depth: 16,
    }),
    "compile-report.json": json({
      bounds: compiled.bounds,
      warnings: compiled.warnings,
      editor_document: `editor/${name}.rhlos-map.json`,
    }),
    "README.txt": strToU8(
      `Install this ZIP in the game's configured mods directory and select ${compiled.details.title} from Custom Missions. The base game datadir supplies characters and other shared resources.\n\nReopen editor/${name}.rhlos-map.json in the level editor with the referenced pinned asset library. This is the editable scene at export time, including unsaved edits; the ZIP does not duplicate the library models and textures.\n\n${compiled.warnings.join("\n")}\n`,
    ),
  };
  return new Promise((resolve, reject) =>
    zip(files, { level: 0 }, (error, bytes) => (error ? reject(error) : resolve(bytes))),
  );
}
