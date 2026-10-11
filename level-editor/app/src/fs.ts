// Shared directory-handle helpers for HTTP originals and browser-local saves.
export async function writeText(
  dir: FileSystemDirectoryHandle,
  name: string,
  text: string,
): Promise<void> {
  const fh = await dir.getFileHandle(name, { create: true });
  const w = await fh.createWritable();
  try {
    await w.write(text);
    await w.close();
  } catch (error) {
    try {
      await w.abort();
    } catch {
      /* stream may already have closed after failing */
    }
    throw new Error(`Cannot write ${name}`, { cause: error });
  }
}

export async function subdir(
  root: FileSystemDirectoryHandle,
  path: string[],
): Promise<FileSystemDirectoryHandle | null> {
  let dir = root;
  for (const part of path) {
    try {
      dir = await dir.getDirectoryHandle(part);
    } catch (error) {
      if (isNotFound(error)) return null;
      throw new Error(`Cannot open directory ${path.join("/")}`, {
        cause: error,
      });
    }
  }
  return dir;
}

export function isNotFound(error: unknown): boolean {
  return error instanceof DOMException && error.name === "NotFoundError";
}

const jsonReaders = new WeakMap<FileSystemDirectoryHandle, Map<string, () => Promise<unknown>>>();

/** Virtual files may already be parsed; avoid serializing them just to read them back. */
export function setJsonReader(
  directory: FileSystemDirectoryHandle,
  name: string,
  read: () => Promise<unknown>,
) {
  let readers = jsonReaders.get(directory);
  if (!readers) jsonReaders.set(directory, (readers = new Map()));
  readers.set(name, read);
}

export async function readJson<T>(dir: FileSystemDirectoryHandle, name: string): Promise<T> {
  const reader = jsonReaders.get(dir)?.get(name);
  if (reader) return (await reader()) as T;
  const fh = await dir.getFileHandle(name);
  const file = await fh.getFile();
  try {
    return JSON.parse(await file.text()) as T;
  } catch (error) {
    throw new Error(`Invalid JSON in ${name}`, { cause: error });
  }
}

export async function listFiles(dir: FileSystemDirectoryHandle): Promise<string[]> {
  const names: string[] = [];
  for await (const [name, entry] of dir.entries()) {
    if (entry.kind === "file") names.push(name);
  }
  return names;
}
