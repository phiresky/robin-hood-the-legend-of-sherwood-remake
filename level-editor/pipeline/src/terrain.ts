// sharp I/O wrapper around the shared terrain core (@rle/shared terrain.ts).
import path from "node:path";
import sharp from "sharp";
import type { Point, ProtoLevel } from "@rle/shared";
import {
  renderTerrainCore,
  type SwatchData,
  type SwatchRole,
  type TerrainSpec,
} from "@rle/shared";
import { libraryDir } from "./env.ts";
import { readOptionalImage } from "./inputs.ts";

export type { TerrainSpec, Road, TerrainRegion } from "@rle/shared";

export async function loadSwatch(id: string, directory = libraryDir): Promise<SwatchData | null> {
  const file = path.join(directory, id, "day.png");
  const bytes = await readOptionalImage(file);
  if (bytes === undefined) return null;
  try {
    const { data, info } = await sharp(bytes).removeAlpha().toColourspace("srgb")
      .raw().toBuffer({ resolveWithObject: true });
    return {
      data: new Uint8Array(data),
      width: info.width,
      height: info.height,
    };
  } catch (error) {
    throw new Error(`cannot decode terrain swatch ${file}`, { cause: error });
  }
}

export interface TerrainResult {
  png: Buffer;
  scatterPoints: Point[];
}

export async function renderTerrain(
  level: ProtoLevel,
  W: number,
  H: number,
  spec: TerrainSpec,
  swatchIds: Record<SwatchRole, string>,
): Promise<TerrainResult | null> {
  const swatches: Partial<Record<SwatchRole, SwatchData>> = {};
  for (const role of Object.keys(swatchIds) as SwatchRole[]) {
    // spec-level swatch bindings override the per-map defaults
    const id = spec.swatches?.[role] ?? swatchIds[role];
    const sw = await loadSwatch(id);
    if (sw) swatches[role] = sw;
  }
  const result = renderTerrainCore({
    W,
    H,
    materialSectors: level.material_sectors.map((ms) => ({
      material: ms.material,
      points: ms.polygon.points,
    })),
    spec,
    swatches,
  });
  if (!result) return null;
  const png = await sharp(Buffer.from(result.rgb), {
    raw: { width: W, height: H, channels: 3 },
  })
    .png()
    .toBuffer();
  return { png, scatterPoints: result.scatterPoints };
}
