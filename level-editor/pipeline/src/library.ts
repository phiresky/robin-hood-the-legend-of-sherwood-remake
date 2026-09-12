import fs from "node:fs/promises";
import path from "node:path";
import type { AssetDescriptor, LibraryIndexEntry } from "@rle/shared";
import { parseAssetDescriptor } from "@rle/shared";
import { readDocument } from "./inputs.ts";
import { libraryDir } from "./env.ts";
import { isMissing } from "./provider-cache.ts";

/** One library's read/modify/publish boundary. The index is atomically replaced;
 * image and descriptor updates are NOT a whole-asset transaction. */
export class AssetLibrary {
  private readonly directory: string;
  private pending: Promise<unknown> = Promise.resolve();

  constructor(directory: string) {
    this.directory = path.resolve(directory);
  }

  writeAsset(
    desc: AssetDescriptor,
    images: Record<string, Buffer>,
  ): Promise<string> {
    const run = this.pending.then(() => this.publish(desc, images));
    // Keep same-owner calls ordered even when a previous publication failed.
    this.pending = run.catch(() => undefined);
    return run;
  }

  private async publish(
    desc: AssetDescriptor,
    images: Record<string, Buffer>,
  ): Promise<string> {
    const next = indexEntry(desc);
    validateIndex([next]);
    for (const name of Object.keys(images)) requireFilename(name, "image name");
    await fs.mkdir(this.directory, { recursive: true });
    const lease = path.join(this.directory, ".index.lock");
    try {
      await fs.mkdir(lease);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "EEXIST") {
        // Never infer that another writer is dead from a timer or stale PID.
        throw new Error(
          `Library publication is locked: ${lease}. Retry after the other writer exits. If a writer crashed, verify that no writer is active, then remove this empty lock directory and retry.`,
          { cause: error },
        );
      }
      throw new Error(`Cannot acquire library publication lock ${lease}`, {
        cause: error,
      });
    }
    try {
      const indexPath = path.join(this.directory, "index.json");
      const entries = (await readLibraryIndex(this.directory)).filter(
        (entry) => entry.id !== desc.id,
      );
      entries.push(next);
      entries.sort((a, b) => a.id.localeCompare(b.id));

      // Validate the old index before touching existing assets: unreadable or
      // malformed metadata is not an invitation to start a new library.
      const dir = path.join(this.directory, desc.id);
      await fs.mkdir(dir, { recursive: true });
      for (const [name, bytes] of Object.entries(images))
        await fs.writeFile(path.join(dir, name), bytes);
      await fs.writeFile(
        path.join(dir, "asset.json"),
        JSON.stringify(desc, null, 2),
      );

      const staging = await fs.mkdtemp(
        path.join(this.directory, ".index-publish-"),
      );
      try {
        const temporaryIndex = path.join(staging, "index.json");
        await fs.writeFile(temporaryIndex, JSON.stringify(entries, null, 2), {
          flag: "wx",
        });
        await fs.rename(temporaryIndex, indexPath);
      } finally {
        // Only the fresh private staging directory belongs to this invocation.
        await fs.rm(staging, { recursive: true, force: true });
      }
      return dir;
    } finally {
      // Acquisition failures never enter this block and cannot remove another
      // process's lease. A non-empty lease fails visibly instead of being erased.
      await fs.rmdir(lease);
    }
  }
}

function requireFilename(value: string, label: string): void {
  if (
    !value ||
    value === "." ||
    value === ".." ||
    /[\\/\0]/.test(value) ||
    value === ".index.lock" ||
    value === "index.json" ||
    value.startsWith(".index-publish-")
  )
    throw new Error(`Invalid ${label}: ${value}`);
}

export async function readAssetDescriptor(file: string): Promise<AssetDescriptor>;
export async function readAssetDescriptor(file: string, required: false): Promise<AssetDescriptor | undefined>;
export async function readAssetDescriptor(file: string, required = true): Promise<AssetDescriptor | undefined> {
  const value = await readDocument(file, required);
  if (value === undefined) return undefined;
  try {
    return parseAssetDescriptor(value);
  } catch (error) {
    throw new Error(`invalid asset descriptor ${file}`, { cause: error });
  }
}

function indexEntry(desc: AssetDescriptor): LibraryIndexEntry {
  return {
    id: desc.id,
    name: desc.name,
    tags: desc.tags,
    scale_class: desc.scale_class,
    source_map: desc.source.map,
    bbox: desc.source.bbox,
  };
}

function validateIndex(value: unknown): asserts value is LibraryIndexEntry[] {
  if (!Array.isArray(value))
    throw new Error("expected an array of library entries");
  const ids = new Set<string>();
  for (const [position, entry] of value.entries()) {
    if (
      !entry ||
      typeof entry !== "object" ||
      Array.isArray(entry) ||
      typeof entry.id !== "string" ||
      typeof entry.name !== "string" ||
      !Array.isArray(entry.tags) ||
      !entry.tags.every((tag: unknown) => typeof tag === "string") ||
      !["unique", "variant", "spline-segment", "texture"].includes(
        entry.scale_class,
      ) ||
      typeof entry.source_map !== "string" ||
      !Array.isArray(entry.bbox) ||
      entry.bbox.length !== 4 ||
      !entry.bbox.every(
        (v: unknown) => typeof v === "number" && Number.isFinite(v),
      )
    )
      throw new Error(`invalid library entry at index ${position}`);
    requireFilename(entry.id, "asset ID");
    if (ids.has(entry.id))
      throw new Error(`duplicate library asset ID ${entry.id}`);
    ids.add(entry.id);
  }
}

/** Atomic index replacement lets readers take a validated snapshot without
 * holding the writer lease across expensive extraction/provider work. */
export async function readLibraryIndex(
  directory = libraryDir,
  required = false,
): Promise<LibraryIndexEntry[]> {
  const file = path.join(directory, "index.json");
  let text: string;
  try {
    text = await fs.readFile(file, "utf8");
  } catch (error) {
    if (!required && isMissing(error)) return [];
    throw new Error(`Cannot read library index ${file}`, { cause: error });
  }
  try {
    const entries: unknown = JSON.parse(text);
    validateIndex(entries);
    return entries;
  } catch (error) {
    throw new Error(`Invalid library index ${file}`, { cause: error });
  }
}

const defaultLibrary = new AssetLibrary(libraryDir);

export async function writeAsset(
  desc: AssetDescriptor,
  images: Record<string, Buffer>,
): Promise<string> {
  return defaultLibrary.writeAsset(desc, images);
}
