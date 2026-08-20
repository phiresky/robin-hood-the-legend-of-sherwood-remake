// Compose mode: MapDraft documents plus the per-asset image caches used for
// canvas drawing and alpha hit-testing. Drafts only exist via explicit
// open/save through the File System Access API pickers.
import type { MapDraft, Placement } from "@rle/shared";
import { loadAssetImage, type LibraryAsset, type LibraryIndex } from "./library";

export const DEFAULT_DRAFT_SIZE: [number, number] = [3136, 1984];
export const DEFAULT_BACKGROUND = "#2a3324";

export function newDraft(name: string, size: [number, number]): MapDraft {
  return {
    version: 1,
    name,
    size,
    background_color: DEFAULT_BACKGROUND,
    placements: [],
  };
}

export function assetById(lib: LibraryIndex | null, id: string): LibraryAsset | null {
  if (!lib) return null;
  return lib.assets.find((a) => a.descriptor.id === id) ?? null;
}

/** world Y of a placement's anchor — the draw-order sort key */
export function anchorY(p: Placement, asset: LibraryAsset | null): number {
  const s = p.scale ?? 1;
  return p.pos[1] + (asset ? asset.descriptor.anchor[1] * s : 0);
}

/** placement indices in draw order (ascending world anchor Y) */
export function drawOrder(draft: MapDraft, lib: LibraryIndex | null): number[] {
  const keys = draft.placements.map((p) => anchorY(p, assetById(lib, p.asset)));
  return [...draft.placements.keys()].sort((a, b) => keys[a]! - keys[b]!);
}

// --- image cache -----------------------------------------------------------

const bitmaps = new Map<string, ImageBitmap | "loading">();

/**
 * Full-size day image for canvas drawing. Returns null while loading; the
 * onLoad callback fires once the bitmap is ready (schedule a redraw there).
 */
export function assetBitmap(asset: LibraryAsset, onLoad: () => void): ImageBitmap | null {
  const id = asset.descriptor.id;
  const cached = bitmaps.get(id);
  if (cached === "loading") return null;
  if (cached) return cached;
  bitmaps.set(id, "loading");
  void loadAssetImage(asset, asset.descriptor.images.day)
    .then((img) => {
      bitmaps.set(id, img);
      onLoad();
    })
    .catch((e) => {
      bitmaps.delete(id);
      console.warn(`failed to load image for ${id}:`, e);
    });
  return null;
}

// --- alpha hit-testing -----------------------------------------------------

// alpha channel per asset id, extracted once from the day bitmap
const alphas = new Map<string, { data: Uint8Array; w: number; h: number }>();

/** true when the asset-local pixel (x, y) is opaque enough to grab */
export function hitAlpha(id: string, bmp: ImageBitmap, x: number, y: number): boolean {
  if (x < 0 || y < 0 || x >= bmp.width || y >= bmp.height) return false;
  let entry = alphas.get(id);
  if (!entry) {
    const cv = document.createElement("canvas");
    cv.width = bmp.width;
    cv.height = bmp.height;
    const ctx = cv.getContext("2d", { willReadFrequently: true })!;
    ctx.drawImage(bmp, 0, 0);
    const rgba = ctx.getImageData(0, 0, bmp.width, bmp.height).data;
    const data = new Uint8Array(bmp.width * bmp.height);
    for (let i = 0; i < data.length; i++) data[i] = rgba[i * 4 + 3]!;
    entry = { data, w: bmp.width, h: bmp.height };
    alphas.set(id, entry);
  }
  return entry.data[(y | 0) * entry.w + (x | 0)]! > 16;
}

// --- file I/O --------------------------------------------------------------

const DRAFT_TYPES = [
  { description: "map draft", accept: { "application/json": [".json"] as string[] } },
];

export async function openDraftFile(): Promise<{
  draft: MapDraft;
  handle: FileSystemFileHandle;
}> {
  const [handle] = await window.showOpenFilePicker({ types: DRAFT_TYPES });
  const file = await handle!.getFile();
  const draft = JSON.parse(await file.text()) as MapDraft;
  if (draft.version !== 1 || !Array.isArray(draft.placements))
    throw new Error(`${file.name}: not a version-1 map draft`);
  return { draft, handle: handle! };
}

/** save to the given handle, or prompt for one; returns the handle for re-save */
export async function saveDraftFile(
  draft: MapDraft,
  handle: FileSystemFileHandle | null,
): Promise<FileSystemFileHandle> {
  const target =
    handle ??
    (await window.showSaveFilePicker({
      suggestedName: `${draft.name.replace(/[^\w-]+/g, "_") || "draft"}.json`,
      types: DRAFT_TYPES,
    }));
  const writable = await target.createWritable();
  await writable.write(JSON.stringify(draft, null, 2));
  await writable.close();
  return target;
}
