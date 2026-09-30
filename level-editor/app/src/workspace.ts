import { createTerrainGrid, expandTerrainGrid, type Level3D } from "@rle/shared";

/** Dimensions use the same map pixels as the saved scenes and export frame. */
export const MAP_SIZE_PRESETS = [
  { name: "York", size: [3136, 2318] },
  { name: "Croisement01", size: [1408, 960] },
  { name: "Croisement02", size: [1792, 1152] },
  { name: "Croisement03", size: [1408, 960] },
  { name: "Derby", size: [1920, 2752] },
  { name: "Leicester", size: [3136, 1984] },
  { name: "Lincoln", size: [2944, 2176] },
  { name: "Nottingham", size: [2304, 3520] },
  { name: "Sherwood", size: [1920, 1088] },
] as const;

export interface NewMapOptions {
  size: [number, number];
  spacing: number;
  height: number;
}

export function defaultNewMapOptions(): NewMapOptions {
  return { size: [1920, 1088], spacing: 128, height: 0 };
}

export function validateWorkspaceSize(size: readonly number[]): asserts size is [number, number] {
  if (size.length !== 2 || size.some((value) => !Number.isSafeInteger(value) || value <= 0))
    throw new Error("Workspace width and height must be positive whole pixels.");
}

export function validateNewMapOptions(
  options: NewMapOptions,
  rowSpacing = options.spacing * Math.sin((35 * Math.PI) / 180),
): void {
  validateWorkspaceSize(options.size);
  if (!Number.isFinite(options.spacing) || options.spacing <= 0)
    throw new Error("Grid spacing must be greater than zero.");
  if (!Number.isFinite(options.height))
    throw new Error("Initial elevation must be a finite number.");
  const columns = Math.ceil(options.size[0] / options.spacing);
  if (!Number.isFinite(rowSpacing) || rowSpacing <= 0)
    throw new Error("Grid row spacing must be greater than zero.");
  const rows = Math.ceil(options.size[1] / rowSpacing);
  if ((columns + 1) * (rows + 1) > 250_000)
    throw new Error("This grid would exceed 250,000 vertices. Increase the grid spacing.");
}

/** Bounds are a non-destructive frame: old terrain and placements remain editable outside it. */
export function resizeWorkspace(document: Level3D, size: [number, number]): Level3D {
  validateWorkspaceSize(size);
  const terrain = document.terrain;
  const previous = document.size;
  const growing = !previous || size[0] > previous[0] || size[1] > previous[1];
  if (growing && terrain)
    validateNewMapOptions(
      { size, spacing: terrain.spacing, height: 0 },
      terrain.rowSpacing ?? terrain.spacing,
    );
  return {
    ...document,
    size: [...size],
    terrain: terrain
      ? growing
        ? expandTerrainGrid(terrain, [0, 0, ...size])
        : terrain
      : document.sceneAssets.some((asset) => asset.role === "ground")
        ? undefined
        : createTerrainGrid(
            [0, 0, ...size],
            128,
            0,
            "grass_short",
            128 * Math.sin((document.camera.elevation_deg * Math.PI) / 180),
          ),
  };
}
