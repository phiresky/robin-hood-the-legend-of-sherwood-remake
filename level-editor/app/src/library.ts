// Scanning and loading of the asset library folder (level-editor/library/).
// The library root holds index.json plus one directory per asset with
// asset.json, day.png, mask.png and optional night.png / fog.png.
import type { AssetDescriptor, LibraryIndexEntry } from "@rle/shared";
import { readImage, readJson, subdir } from "./fs";

export interface LibraryAsset {
  descriptor: AssetDescriptor;
  /** object URL for day.png (thumbnail + preview) */
  dayUrl: string;
  /** object URL for night.png, if present */
  nightUrl: string | null;
  dir: FileSystemDirectoryHandle;
}

export interface LibraryIndex {
  assets: LibraryAsset[];
  root: FileSystemDirectoryHandle;
}

async function fileUrl(
  dir: FileSystemDirectoryHandle,
  name: string,
): Promise<string | null> {
  try {
    const fh = await dir.getFileHandle(name);
    return URL.createObjectURL(await fh.getFile());
  } catch {
    return null;
  }
}

export async function scanLibrary(root: FileSystemDirectoryHandle): Promise<LibraryIndex> {
  const entries = await readJson<LibraryIndexEntry[]>(root, "index.json");
  const assets: LibraryAsset[] = [];
  for (const entry of entries) {
    const dir = await subdir(root, [entry.id]);
    if (!dir) {
      console.warn(`library index lists ${entry.id} but the directory is missing`);
      continue;
    }
    const descriptor = await readJson<AssetDescriptor>(dir, "asset.json");
    const dayUrl = await fileUrl(dir, descriptor.images.day);
    if (!dayUrl) throw new Error(`${entry.id}: missing ${descriptor.images.day}`);
    const nightUrl = descriptor.images.night
      ? await fileUrl(dir, descriptor.images.night)
      : null;
    assets.push({ descriptor, dayUrl, nightUrl, dir });
  }
  return { assets, root };
}

export function releaseLibrary(index: LibraryIndex) {
  for (const a of index.assets) {
    URL.revokeObjectURL(a.dayUrl);
    if (a.nightUrl) URL.revokeObjectURL(a.nightUrl);
  }
}

/** filter over name + tags, case-insensitive, every whitespace-separated term must match */
export function filterAssets(assets: LibraryAsset[], query: string): LibraryAsset[] {
  const terms = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (terms.length === 0) return assets;
  return assets.filter((a) => {
    const hay = `${a.descriptor.name} ${a.descriptor.tags.join(" ")}`.toLowerCase();
    return terms.every((t) => hay.includes(t));
  });
}

/** group assets by variant_group (solo assets form their own group), sorted by name */
export function groupAssets(assets: LibraryAsset[]): [string, LibraryAsset[]][] {
  const groups = new Map<string, LibraryAsset[]>();
  for (const a of assets) {
    const key = a.descriptor.variant_group ?? a.descriptor.id;
    const list = groups.get(key);
    if (list) list.push(a);
    else groups.set(key, [a]);
  }
  return [...groups.entries()].sort(([a], [b]) => a.localeCompare(b));
}

export async function loadAssetImage(
  asset: LibraryAsset,
  name: string,
): Promise<ImageBitmap> {
  return readImage(asset.dir, name);
}
