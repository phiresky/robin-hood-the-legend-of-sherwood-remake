import { publishedMapLabel } from "./map-label.ts";
import { loadHttpAssetCatalog, loadHttpModelParts } from "./http-asset-catalog.ts";
import { writeMapThumbnail, thumbnailExtensions } from "./map-thumbnail";
import { validateNewMap } from "./new-map";
import { isNotFound, listFiles, writeText } from "./fs.ts";
import { Temporal } from "temporal-polyfill";

async function migrateBrowserMaps(maps: FileSystemDirectoryHandle) {
  const legacyExtension = ".level3d.json";
  const pending: string[] = [];
  for await (const [name, entry] of maps.entries())
    if (entry.kind === "file" && name.endsWith(legacyExtension)) pending.push(name);
  for (const name of pending) {
    const destination = name.slice(0, -legacyExtension.length) + ".rhlos-map.json";
    try {
      await maps.getFileHandle(destination);
      continue; // An existing new-format save takes precedence; retain the older file for recovery.
    } catch (error) {
      if (!isNotFound(error)) throw error;
    }
    const source = await (await maps.getFileHandle(name)).getFile();
    const output = await maps.getFileHandle(destination, { create: true });
    const writer = await output.createWritable();
    try {
      await writer.write(source);
      await writer.close();
    } catch (error) {
      await writer.abort().catch(() => {});
      await maps.removeEntry(destination);
      throw error;
    }
    await maps.removeEntry(name);
  }
}

const missing = (path: string) =>
  new DOMException(`Missing library file: ${path}`, "NotFoundError");
function segment(name: string) {
  if (!name || name === "." || name === ".." || /[\\/\0]/.test(name))
    throw new Error("Invalid library path");
  return name;
}

/** Read-only HTTP originals and separately selectable browser-local map copies. */
export async function openHttpLibrary(
  base = (import.meta.env?.BASE_URL ?? "/") + "library/",
  storage?: FileSystemDirectoryHandle,
) {
  let catalog = await loadHttpAssetCatalog(base);
  const browser = storage ?? (await navigator.storage.getDirectory());
  const workspace = await browser.getDirectoryHandle("sherwood-level-editor", { create: true });
  const maps = await workspace.getDirectoryHandle("maps", { create: true });
  await migrateBrowserMaps(maps);
  const remoteFile = async (path: string) => {
    if (path === "3d-assets/index.json") {
      // Maps and their descriptor pins can change while this connection is open.
      // Refresh the manifest too, including the model chunks for the new release.
      catalog = await loadHttpAssetCatalog(base);
      return new File([JSON.stringify(catalog)], "index.json", { type: "application/json" });
    }
    if (catalog.model_shards?.[path])
      return new File(
        [await loadHttpModelParts(base, catalog.model_shards[path])],
        path.split("/").at(-1)!,
        { type: "model/gltf-binary" },
      );
    const result = await fetch(base + path.split("/").map(encodeURIComponent).join("/"), {
      cache: "no-cache",
    });
    if (result.status === 404) throw missing(path);
    if (!result.ok) throw new Error(`Cannot read ${path} (${result.status})`);
    if (result.headers.get("content-type")?.includes("text/html")) throw missing(path);
    return new File([await result.arrayBuffer()], path.split("/").at(-1)!, {
      type: result.headers.get("content-type") ?? "",
    });
  };
  const published: unknown = JSON.parse(await (await remoteFile("scenes/index.json")).text());
  if (
    !Array.isArray(published) ||
    published.some((name) => typeof name !== "string" || !name.endsWith(".rhlos-map.json"))
  )
    throw new Error("Invalid map index");
  const publishedNames = published.map((name) => segment(name as string));
  const publishedFiles = new Set(publishedNames);
  const extension = ".rhlos-map.json";
  const modifiedSuffix = " (Modified)";
  // Copy labels are virtual names; stored map names and document IDs stay unchanged.
  const copySource = (name: string) => {
    const match = /^(.*) \(Modified(?: [2-9][0-9]*| 1[0-9]+)?\)$/.exec(name);
    return match && publishedFiles.has(match[1]! + extension) ? match[1]! : null;
  };
  const documentMap = (name: string) => copySource(name) ?? name;
  const storageName = (name: string) =>
    name.endsWith(modifiedSuffix) ? (copySource(name) ?? name) : name;
  const savedMapName = (name: string) =>
    publishedFiles.has(name + extension) ? name + modifiedSuffix : name;
  const isBuiltIn = (name: string) => publishedFiles.has(name + extension);
  async function availableMapName(name: string) {
    if (!isBuiltIn(name)) return name;
    const existing = new Set<string>();
    for await (const [file] of maps.entries()) existing.add(file.toLowerCase());
    let copy = name + modifiedSuffix;
    for (
      let n = 2;
      existing.has((storageName(copy) + extension).toLowerCase()) ||
      publishedFiles.has(copy + extension);
      n++
    )
      copy = `${name} (Modified ${n})`;
    return copy;
  }
  async function saveMap(name: string, document: unknown, thumbnail: Blob) {
    return navigator.locks.request("sherwood-map-files", async () => {
      const target = await availableMapName(name);
      const file = await maps.getFileHandle(storageName(target) + extension, { create: true });
      const writer = await file.createWritable();
      try {
        await writer.write(JSON.stringify(document, null, 2));
        await writer.close();
      } catch (error) {
        await writer.abort().catch(() => {});
        throw error;
      }
      await writeMapThumbnail(maps, storageName(target), thumbnail);
      return target;
    });
  }
  async function deleteMap(name: string) {
    if (isBuiltIn(name)) throw new Error("Built-in maps cannot be deleted");
    await navigator.locks.request("sherwood-map-files", async () => {
      await maps.removeEntry(storageName(name) + extension);
      for (const ext of thumbnailExtensions) {
        try {
          await maps.removeEntry(`${storageName(name)}.${ext}`);
        } catch (error) {
          if (!isNotFound(error)) throw error;
        }
      }
    });
  }
  async function renameMap(name: string, rawName: string) {
    if (isBuiltIn(name)) throw new Error("Built-in maps cannot be renamed");
    const next = validateNewMap(rawName);
    if (next === name) return next;
    return navigator.locks.request("sherwood-map-files", async () => {
      const entries = await listFiles(directory("scenes/"));
      if (entries.some((entry) => entry.toLowerCase() === (next + extension).toLowerCase()))
        throw new Error(`A map named “${next}” already exists.`);
      const oldName = storageName(name);
      const document = JSON.parse(
        await (await (await maps.getFileHandle(oldName + extension)).getFile()).text(),
      );
      document.map = next;
      const created: string[] = [];
      try {
        for (const ext of thumbnailExtensions) {
          let preview: File;
          try {
            preview = await (await maps.getFileHandle(`${oldName}.${ext}`)).getFile();
          } catch (error) {
            if (isNotFound(error)) continue;
            throw error;
          }
          const writer = await (
            await maps.getFileHandle(`${next}.${ext}`, { create: true })
          ).createWritable();
          created.push(`${next}.${ext}`);
          await writer.write(preview);
          await writer.close();
        }
        created.push(next + extension);
        await writeText(maps, next + extension, JSON.stringify(document, null, 2));
      } catch (error) {
        for (const file of created) await maps.removeEntry(file).catch(() => {});
        throw error;
      }
      await maps.removeEntry(oldName + extension);
      for (const ext of thumbnailExtensions) {
        try {
          await maps.removeEntry(`${oldName}.${ext}`);
        } catch (error) {
          if (!isNotFound(error)) throw error;
        }
      }
      return next;
    });
  }
  async function mapLabels() {
    const local = new Set<string>();
    for await (const [name, entry] of maps.entries()) if (entry.kind === "file") local.add(name);
    const labels = new Map<string, string>();
    for (const file of publishedNames) {
      const name = file.slice(0, -extension.length);
      labels.set(name, publishedMapLabel(name, false));
      if (local.has(file)) labels.set(savedMapName(name), publishedMapLabel(name, true));
    }
    for (const file of local) {
      if (!file.endsWith(extension)) continue;
      const name = file.slice(0, -extension.length);
      const source = copySource(name);
      if (source) labels.set(name, publishedMapLabel(source, false) + name.slice(source.length));
    }
    return labels;
  }
  function directory(prefix: string): FileSystemDirectoryHandle {
    const isMaps = prefix === "scenes/";
    return {
      kind: "directory",
      name: prefix ? prefix.split("/").at(-2)! : "HTTP library",
      async getDirectoryHandle(name: string) {
        const next = prefix + segment(name) + "/";
        return directory(next);
      },
      async getFileHandle(name: string, options?: FileSystemGetFileOptions) {
        const path = prefix + segment(name);
        const thumbnailExtension = [".avif", ".webp", ".png"].find((extension) =>
          name.endsWith(extension),
        );
        if (isMaps && thumbnailExtension) {
          const id = name.slice(0, -thumbnailExtension.length);
          const source = storageName(id);
          if (options?.create || source !== id || !publishedFiles.has(source + extension))
            return maps.getFileHandle(source + thumbnailExtension, options);
          const file = await remoteFile(path);
          return { kind: "file", name, getFile: async () => file } as FileSystemFileHandle;
        }
        if (isMaps && name.endsWith(".rhlos-map.json")) {
          const id = name.slice(0, -extension.length);
          const source = storageName(id);
          if (source !== id) return maps.getFileHandle(source + extension, options);
          if (!publishedFiles.has(name)) return maps.getFileHandle(name, options);
          return {
            kind: "file",
            name,
            getFile: async () => remoteFile(path),
            async createWritable() {
              return (await maps.getFileHandle(name, { create: true })).createWritable();
            },
          } as FileSystemFileHandle;
        }
        if (options?.create)
          throw new DOMException("Library assets are read-only", "NotAllowedError");
        const file = await remoteFile(path);
        return { kind: "file", name, getFile: async () => file } as FileSystemFileHandle;
      },
      async *entries() {
        const names = new Map<string, "file" | "directory">();
        if (!isMaps)
          throw new Error("Only map enumeration is supported; use the asset index for assets");
        for (const name of publishedNames) names.set(name, "file");
        for await (const [name, entry] of maps.entries())
          if (entry.kind === "file" && name.endsWith(extension))
            names.set(savedMapName(name.slice(0, -extension.length)) + extension, "file");
        for (const [name, kind] of names)
          yield [name, kind === "directory" ? directory(prefix + name + "/") : { kind, name }] as [
            string,
            FileSystemHandle,
          ];
      },
      async removeEntry(name: string) {
        const suffix = [extension, ".avif", ".webp", ".png"].find((suffix) =>
          segment(name).endsWith(suffix),
        );
        if (!isMaps || !suffix)
          throw new DOMException("Library assets are read-only", "NotAllowedError");
        await maps.removeEntry(storageName(name.slice(0, -suffix.length)) + suffix);
      },
    } as unknown as FileSystemDirectoryHandle;
  }
  return {
    handle: directory(""),
    mapLabels,
    documentMap,
    savedMapName,
    availableMapName,
    saveMap,
    isBuiltIn,
    deleteMap,
    renameMap,
  };
}

export function downloadMap(name: string, document: unknown) {
  const now = Temporal.Now.zonedDateTimeISO();
  const pad = (value: number) => String(value).padStart(2, "0");
  const timestamp = `${now.year}-${pad(now.month)}-${pad(now.day)}T${pad(now.hour)}-${pad(now.minute)}-${pad(now.second)}`;
  const url = URL.createObjectURL(
    new Blob([JSON.stringify(document, null, 2) + "\n"], { type: "application/json" }),
  );
  const link = window.document.createElement("a");
  link.href = url;
  link.download = `${name}_${timestamp}.rhlos-map.json`;
  link.click();
  setTimeout(() => URL.revokeObjectURL(url), 0);
}
