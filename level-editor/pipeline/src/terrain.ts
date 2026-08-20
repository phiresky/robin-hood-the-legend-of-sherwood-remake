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
import { libraryDir } from "./env";

export type { TerrainSpec, Road, TerrainRegion } from "@rle/shared";

export async function loadSwatch(id: string): Promise<SwatchData | null> {
  try {
    const img = sharp(path.join(libraryDir, id, "day.png")).removeAlpha();
    const { width, height } = await img.metadata();
    return {
      data: new Uint8Array(await img.raw().toBuffer()),
      width: width!,
      height: height!,
    };
  } catch {
    return null;
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
