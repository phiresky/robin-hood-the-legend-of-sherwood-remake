// Compose mode: MapDraft documents plus the per-asset image caches used for
// canvas drawing and alpha hit-testing. Drafts only exist via explicit
// open/save through the File System Access API pickers.
import type { MapDraft, Placement } from "@rle/shared";
import { loadAssetImage, type LibraryAsset, type LibraryIndex } from "./library";

export const DEFAULT_BACKGROUND = "#2a3324";

/** margin added around the content bounding box when computing size at save */
export const SAVE_MARGIN = 64;

/**
 * Editing happens on an infinite canvas; `size` is only computed at save time
 * from the placement bounds, so a fresh draft starts with a zero size.
 */
export function newDraft(name: string): MapDraft {
  return {
    version: 1,
    name,
    size: [0, 0],
    background_color: DEFAULT_BACKGROUND,
    placements: [],
  };
}

/** world-space rect [x, y, w, h] covered by a placement's image */
export function placementRect(
  p: Placement,
  asset: LibraryAsset | null,
): [number, number, number, number] | null {
  if (!asset) return null;
  const s = p.scale ?? 1;
  const [, , w, h] = asset.descriptor.source.bbox;
  return [p.pos[0], p.pos[1], w * s, h * s];
}

/** bounding box [minX, minY, maxX, maxY] of all placements, or null if empty */
export function contentBounds(
  draft: MapDraft,
  lib: LibraryIndex | null,
): [number, number, number, number] | null {
  let bounds: [number, number, number, number] | null = null;
  for (const p of draft.placements) {
    const r = placementRect(p, assetById(lib, p.asset));
    if (!r) continue;
    const [x, y, w, h] = r;
    if (!bounds) bounds = [x, y, x + w, y + h];
    else {
      bounds[0] = Math.min(bounds[0], x);
      bounds[1] = Math.min(bounds[1], y);
      bounds[2] = Math.max(bounds[2], x + w);
      bounds[3] = Math.max(bounds[3], y + h);
    }
  }
  return bounds;
}

/**
 * Boundary guide rect [x, y, w, h]: the content bounds plus margin (what save
 * would write as `size`), or the stored size for a draft without renderable
 * placements. Null when there is nothing to outline.
 */
export function guideRect(
  draft: MapDraft,
  lib: LibraryIndex | null,
): [number, number, number, number] | null {
  const b = contentBounds(draft, lib);
  if (b)
    return [
      b[0] - SAVE_MARGIN,
      b[1] - SAVE_MARGIN,
      b[2] - b[0] + 2 * SAVE_MARGIN,
      b[3] - b[1] + 2 * SAVE_MARGIN,
    ];
  if (draft.size[0] > 0 && draft.size[1] > 0) return [0, 0, draft.size[0], draft.size[1]];
  return null;
}

/**
 * Snapshot for writing to disk: shift placements so the content (plus margin)
 * starts at (0,0) and set `size` accordingly. The in-memory draft is left
 * untouched so the view and placement coords stay stable while editing.
 */
export function finalizeDraft(draft: MapDraft, lib: LibraryIndex | null): MapDraft {
  const bounds = contentBounds(draft, lib);
  if (!bounds) return { ...draft, size: [0, 0] };
  const [minX, minY, maxX, maxY] = bounds;
  const dx = SAVE_MARGIN - minX;
  const dy = SAVE_MARGIN - minY;
  return {
    ...draft,
    size: [Math.ceil(maxX - minX) + 2 * SAVE_MARGIN, Math.ceil(maxY - minY) + 2 * SAVE_MARGIN],
    placements: draft.placements.map((p) => ({
      ...p,
      pos: [Math.round(p.pos[0] + dx), Math.round(p.pos[1] + dy)],
    })),
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
