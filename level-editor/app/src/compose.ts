// Compose mode: MapDraft documents plus the per-asset image caches used for
// canvas drawing and alpha hit-testing. Drafts only exist via explicit
// open/save through the File System Access API pickers.
import type {
  DirectionalStamp,
  MapDraft,
  Placement,
  Point,
  TerrainRegion,
  WallRun,
  WallSegmentSpec,
} from "@rle/shared";
import { expandWallRun, expandWallRunDirectional } from "@rle/shared";
import { loadAssetImage, type LibraryAsset, type LibraryIndex } from "./library";

/** what is selected in compose mode: a placement, wall run, or terrain shape */
export type DraftSelection =
  | { kind: "placement"; idx: number }
  | { kind: "wall"; idx: number }
  | { kind: "region"; idx: number }
  | { kind: "road"; idx: number }
  | null;

/** active terrain drawing tool */
export type TerrainTool =
  | { kind: "region"; material: TerrainRegion["material"] }
  | { kind: "road"; width: number };

export function selectionEquals(a: DraftSelection, b: DraftSelection): boolean {
  if (a === null || b === null) return a === b;
  return a.kind === b.kind && a.idx === b.idx;
}

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
    walls: [],
  };
}

// --- wall runs -------------------------------------------------------------

function segmentSpec(asset: LibraryAsset): WallSegmentSpec | null {
  const d = asset.descriptor;
  if (d.wall_direction_deg === undefined) return null;
  const [, , w, h] = d.source.bbox;
  return { id: d.id, size: [w, h], anchor: d.anchor, directionDeg: d.wall_direction_deg };
}

/**
 * Direction-aware segment set for a wall started from `asset`: every library
 * asset with `wall_direction_deg` set that shares the source map. Empty when
 * the started asset itself has no direction (single-segment mode).
 */
export function wallSegmentSet(asset: LibraryAsset, lib: LibraryIndex | null): string[] {
  if (!lib || asset.descriptor.wall_direction_deg === undefined) return [];
  return lib.assets
    .filter(
      (a) =>
        a.descriptor.wall_direction_deg !== undefined &&
        a.descriptor.source.map === asset.descriptor.source.map,
    )
    .map((a) => a.descriptor.id);
}

/** expand a point path with the run's segment set (or single-asset fallback) */
function expandRun(
  points: readonly [number, number][],
  assetId: string,
  segmentSet: string[] | undefined,
  spacing: number | undefined,
  lib: LibraryIndex | null,
): DirectionalStamp[] {
  const specs = (segmentSet ?? [])
    .map((id) => {
      const a = assetById(lib, id);
      return a ? segmentSpec(a) : null;
    })
    .filter((s): s is WallSegmentSpec => s !== null);
  if (specs.length > 0) return expandWallRunDirectional(points, specs, spacing);
  const asset = assetById(lib, assetId);
  if (!asset) return [];
  const [, , w, h] = asset.descriptor.source.bbox;
  return expandWallRun(points, [w, h], asset.descriptor.anchor, spacing).map((s) => ({
    ...s,
    asset: assetId,
  }));
}

// Stamp expansion cached per WallRun object; edits replace the run object
// (immutable updates), so identity keying auto-invalidates. A library rescan
// replaces the LibraryIndex object, invalidating too.
const stampCache = new WeakMap<
  WallRun,
  { lib: LibraryIndex | null; stamps: DirectionalStamp[] }
>();

/** stamp positions for a wall run, via the shared stitching helpers (cached) */
export function wallStamps(run: WallRun, lib: LibraryIndex | null): DirectionalStamp[] {
  const cached = stampCache.get(run);
  if (cached && cached.lib === lib) return cached.stamps;
  const stamps = expandRun(run.points, run.asset, run.segment_set, run.spacing, lib);
  stampCache.set(run, { lib, stamps });
  return stamps;
}

/** live preview stamps for an in-progress run started from `asset` (uncached) */
export function previewWallStamps(
  points: readonly [number, number][],
  asset: LibraryAsset,
  lib: LibraryIndex | null,
): DirectionalStamp[] {
  return expandRun(points, asset.descriptor.id, wallSegmentSet(asset, lib), undefined, lib);
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
  const extend = (x: number, y: number, w: number, h: number) => {
    if (!bounds) bounds = [x, y, x + w, y + h];
    else {
      bounds[0] = Math.min(bounds[0], x);
      bounds[1] = Math.min(bounds[1], y);
      bounds[2] = Math.max(bounds[2], x + w);
      bounds[3] = Math.max(bounds[3], y + h);
    }
  };
  for (const p of draft.placements) {
    const r = placementRect(p, assetById(lib, p.asset));
    if (r) extend(...r);
  }
  for (const run of draft.walls ?? []) {
    for (const s of wallStamps(run, lib)) {
      const asset = assetById(lib, s.asset);
      if (!asset) continue;
      const [, , w, h] = asset.descriptor.source.bbox;
      extend(s.pos[0], s.pos[1], w, h);
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
    walls: (draft.walls ?? []).map((r) => ({
      ...r,
      points: r.points.map(
        ([x, y]) => [Math.round(x + dx), Math.round(y + dy)] as [number, number],
      ),
    })),
    ...(draft.terrain
      ? {
          terrain: {
            ...draft.terrain,
            regions: (draft.terrain.regions ?? []).map((rg) => ({
              ...rg,
              polygon: rg.polygon.map(
                ([x, y]) => [Math.round(x + dx), Math.round(y + dy)] as Point,
              ),
            })),
            roads: (draft.terrain.roads ?? []).map((rd) => ({
              ...rd,
              points: rd.points.map(
                ([x, y]) => [Math.round(x + dx), Math.round(y + dy)] as Point,
              ),
            })),
          },
        }
      : {}),
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

export type DrawItem =
  | { kind: "placement"; idx: number }
  | { kind: "stamp"; wall: number; asset: string; pos: [number, number] };

/**
 * Draw list: static content first (placements + wall stamps interleaved by
 * world anchor Y), then FX/patch assets (descriptor `fx` set) on top —
 * in-game, patch roofs draw over the background/buildings.
 */
export function drawItems(draft: MapDraft, lib: LibraryIndex | null): DrawItem[] {
  const keyed: { item: DrawItem; fx: number; sortY: number }[] = [];
  draft.placements.forEach((p, idx) => {
    const asset = assetById(lib, p.asset);
    keyed.push({
      item: { kind: "placement", idx },
      fx: asset?.descriptor.fx ? 1 : 0,
      sortY: anchorY(p, asset),
    });
  });
  (draft.walls ?? []).forEach((run, wall) => {
    for (const s of wallStamps(run, lib))
      keyed.push({
        item: { kind: "stamp", wall, asset: s.asset, pos: s.pos },
        fx: 0,
        sortY: s.sortY,
      });
  });
  keyed.sort((a, b) => a.fx - b.fx || a.sortY - b.sortY);
  return keyed.map((k) => k.item);
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
