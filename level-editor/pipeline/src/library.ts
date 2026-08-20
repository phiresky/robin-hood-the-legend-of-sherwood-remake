import fs from "node:fs/promises";
import path from "node:path";
import type { AssetDescriptor, LibraryIndexEntry } from "@rle/shared";
import { libraryDir } from "./env";

export async function writeAsset(
  desc: AssetDescriptor,
  images: Record<string, Buffer>,
): Promise<string> {
  const dir = path.join(libraryDir, desc.id);
  await fs.mkdir(dir, { recursive: true });
  for (const [name, buf] of Object.entries(images)) {
    await fs.writeFile(path.join(dir, name), buf);
  }
  await fs.writeFile(path.join(dir, "asset.json"), JSON.stringify(desc, null, 2));
  await updateIndex(desc);
  return dir;
}

async function updateIndex(desc: AssetDescriptor) {
  const indexPath = path.join(libraryDir, "index.json");
  let entries: LibraryIndexEntry[] = [];
  try {
    entries = JSON.parse(await fs.readFile(indexPath, "utf8"));
  } catch {
    // first asset
  }
  entries = entries.filter((e) => e.id !== desc.id);
  entries.push({
    id: desc.id,
    name: desc.name,
    tags: desc.tags,
    scale_class: desc.scale_class,
    source_map: desc.source.map,
    bbox: desc.source.bbox,
  });
  entries.sort((a, b) => a.id.localeCompare(b.id));
  await fs.writeFile(indexPath, JSON.stringify(entries, null, 2));
}
