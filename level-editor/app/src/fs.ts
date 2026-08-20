// File System Access API helpers: pick the hackable datadir root, persist the
// handle in IndexedDB, restore it on revisit (Chrome 122+ "allow on every
// visit" makes this prompt-free after the first grant).
import { get, set } from "idb-keyval";

const DATADIR_KEY = "datadir-handle";

export async function pickDatadir(): Promise<FileSystemDirectoryHandle> {
  const handle = await window.showDirectoryPicker({ id: "hackable-datadir", mode: "read" });
  await set(DATADIR_KEY, handle);
  return handle;
}

export async function restoreDatadir(): Promise<FileSystemDirectoryHandle | null> {
  const handle = await get<FileSystemDirectoryHandle>(DATADIR_KEY);
  if (!handle) return null;
  const perm = await handle.queryPermission({ mode: "read" });
  if (perm === "granted") return handle;
  return null; // needs a user gesture; App offers a "reconnect" button
}

export async function requestDatadirPermission(
  handle: FileSystemDirectoryHandle,
): Promise<boolean> {
  return (await handle.requestPermission({ mode: "read" })) === "granted";
}

export async function getStoredDatadirHandle(): Promise<FileSystemDirectoryHandle | null> {
  return (await get<FileSystemDirectoryHandle>(DATADIR_KEY)) ?? null;
}

export async function subdir(
  root: FileSystemDirectoryHandle,
  path: string[],
): Promise<FileSystemDirectoryHandle | null> {
  let dir = root;
  for (const part of path) {
    try {
      dir = await dir.getDirectoryHandle(part);
    } catch {
      return null;
    }
  }
  return dir;
}

export async function readJson<T>(dir: FileSystemDirectoryHandle, name: string): Promise<T> {
  const fh = await dir.getFileHandle(name);
  const file = await fh.getFile();
  return JSON.parse(await file.text()) as T;
}

export async function readImage(
  dir: FileSystemDirectoryHandle,
  name: string,
): Promise<ImageBitmap> {
  const fh = await dir.getFileHandle(name);
  const file = await fh.getFile();
  return createImageBitmap(await file.arrayBuffer().then((b) => new Blob([b])));
}

export async function listFiles(dir: FileSystemDirectoryHandle): Promise<string[]> {
  const names: string[] = [];
  for await (const [name, entry] of dir.entries()) {
    if (entry.kind === "file") names.push(name);
  }
  return names;
}

export async function listDirs(dir: FileSystemDirectoryHandle): Promise<string[]> {
  const names: string[] = [];
  for await (const [name, entry] of dir.entries()) {
    if (entry.kind === "directory") names.push(name);
  }
  return names;
}

/** case-insensitive file lookup (map PNGs are lowercased inconsistently) */
export async function findFileCI(
  dir: FileSystemDirectoryHandle,
  name: string,
): Promise<string | null> {
  const lower = name.toLowerCase();
  for await (const [entry, h] of dir.entries()) {
    if (h.kind === "file" && entry.toLowerCase() === lower) return entry;
  }
  return null;
}
