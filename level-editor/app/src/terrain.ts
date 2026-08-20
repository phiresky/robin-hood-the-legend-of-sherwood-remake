// Terrain preview for compose mode: loads texture swatches from the library
// and runs the shared terrain core at a downscaled resolution over the draft's
// save-bounds area.
import {
  renderTerrainCore,
  type MapDraft,
  type Point,
  type SwatchData,
  type SwatchRole,
} from "@rle/shared";
import { assetById } from "./compose";
import { loadAssetImage, type LibraryIndex } from "./library";

/** preview downscale factor: terrain renders at W/4 × H/4 */
export const TERRAIN_PREVIEW_SCALE = 4;

export const TERRAIN_ROLES: SwatchRole[] = ["grass", "dirt", "road", "canopy", "water"];

/** editor UI colors per material / role (previews, handles, highlights) */
export const MATERIAL_COLORS: Record<string, string> = {
  grass: "#7cb65c",
  dirt: "#a98358",
  road: "#c9a96a",
  canopy: "#3e7a3e",
  water: "#4a7fb5",
};

// raw-RGB swatch data per asset id, extracted once from day.png
const swatchCache = new Map<string, Promise<SwatchData>>();

function loadSwatch(lib: LibraryIndex, id: string): Promise<SwatchData> | null {
  const asset = assetById(lib, id);
  if (!asset) return null;
  let p = swatchCache.get(id);
  if (!p) {
    p = loadAssetImage(asset, asset.descriptor.images.day).then((bmp) => {
      const cv = document.createElement("canvas");
      cv.width = bmp.width;
      cv.height = bmp.height;
      const ctx = cv.getContext("2d", { willReadFrequently: true })!;
      ctx.drawImage(bmp, 0, 0);
      const rgba = ctx.getImageData(0, 0, bmp.width, bmp.height).data;
      const data = new Uint8Array(bmp.width * bmp.height * 3);
      for (let i = 0; i < bmp.width * bmp.height; i++) {
        data[i * 3] = rgba[i * 4]!;
        data[i * 3 + 1] = rgba[i * 4 + 1]!;
        data[i * 3 + 2] = rgba[i * 4 + 2]!;
      }
      return { data, width: bmp.width, height: bmp.height };
    });
    swatchCache.set(id, p);
    p.catch(() => swatchCache.delete(id));
  }
  return p;
}

export interface TerrainLayer {
  canvas: HTMLCanvasElement;
  /** world rect [x, y, w, h] the preview covers */
  rect: [number, number, number, number];
}

/**
 * Render the draft's terrain over the world rect at 1/TERRAIN_PREVIEW_SCALE
 * resolution. Returns null when the draft has no terrain or no grass swatch
 * is bound/loadable (the core needs a grass base).
 */
export async function renderTerrainPreview(
  draft: MapDraft,
  lib: LibraryIndex,
  rect: [number, number, number, number],
): Promise<TerrainLayer | null> {
  const spec = draft.terrain;
  if (!spec) return null;
  const swatches: Partial<Record<SwatchRole, SwatchData>> = {};
  for (const role of TERRAIN_ROLES) {
    const id = spec.swatches?.[role];
    if (!id) continue;
    const p = loadSwatch(lib, id);
    if (!p) continue;
    try {
      swatches[role] = await p;
    } catch (e) {
      console.warn(`terrain swatch ${id} failed to load:`, e);
    }
  }

  const [gx, gy, gw, gh] = rect;
  // the core works in rect-local coords; world points shift by the rect origin
  const shift = (pts: Point[]): Point[] => pts.map(([x, y]) => [x - gx, y - gy]);
  const shifted = {
    roads: (spec.roads ?? []).map((rd) => ({ ...rd, points: shift(rd.points) })),
    regions: (spec.regions ?? []).map((rg) => ({ ...rg, polygon: shift(rg.polygon) })),
    swatches: spec.swatches,
  };
  const W = Math.max(1, Math.ceil(gw / TERRAIN_PREVIEW_SCALE));
  const H = Math.max(1, Math.ceil(gh / TERRAIN_PREVIEW_SCALE));
  // let the pending note paint before the heavy synchronous pass
  await new Promise((r) => setTimeout(r, 0));
  const res = renderTerrainCore({
    W,
    H,
    materialSectors: [],
    spec: shifted,
    swatches,
    scale: TERRAIN_PREVIEW_SCALE,
  });
  if (!res) return null;

  const img = new ImageData(W, H);
  for (let i = 0; i < W * H; i++) {
    img.data[i * 4] = res.rgb[i * 3]!;
    img.data[i * 4 + 1] = res.rgb[i * 3 + 1]!;
    img.data[i * 4 + 2] = res.rgb[i * 3 + 2]!;
    img.data[i * 4 + 3] = 255;
  }
  const cv = document.createElement("canvas");
  cv.width = W;
  cv.height = H;
  cv.getContext("2d")!.putImageData(img, 0, 0);
  return { canvas: cv, rect };
}
