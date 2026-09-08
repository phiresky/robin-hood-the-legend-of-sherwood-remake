import crypto from "node:crypto";
import fs from "node:fs/promises";
import path from "node:path";

export type CacheState<T> =
  | { state: "miss" }
  | { state: "corrupt"; error: unknown }
  | { state: "complete"; value: T };
export interface CacheOptions {
  offline?: boolean;
  workDirectory?: string;
}
const pending = new Map<string, Promise<unknown>>();
const manifestName = "complete.json";

/** Preserve previously paid artifacts while using framed full hashes for new requests. */
export async function cacheDirectory(
  root: string,
  key: string,
  legacyKey: string,
): Promise<string> {
  const legacy = path.join(root, legacyKey);
  try {
    await fs.stat(legacy);
    return legacy;
  } catch (error) {
    if (!isMissing(error)) throw error;
  }
  return path.join(root, key);
}

export function isMissing(error: unknown): boolean {
  return (error as NodeJS.ErrnoException)?.code === "ENOENT";
}

/** Length framing prevents ambiguous concatenations; full SHA-256 keys retain endpoint/version identity. */
export function contentKey(parts: readonly (string | Uint8Array)[]): string {
  const hash = crypto.createHash("sha256");
  for (const part of parts) {
    const data = typeof part === "string" ? Buffer.from(part) : part;
    hash.update(`${data.byteLength}:`).update(data);
  }
  return hash.digest("hex");
}

async function inventory(directory: string): Promise<Record<string, string>> {
  const result: Record<string, string> = {};
  for (const entry of (
    await fs.readdir(directory, { withFileTypes: true })
  ).sort((a, b) => a.name.localeCompare(b.name))) {
    if (entry.name === manifestName) continue;
    if (!entry.isFile())
      throw new Error(`unexpected non-file cache artifact: ${entry.name}`);
    result[entry.name] = crypto
      .createHash("sha256")
      .update(await fs.readFile(path.join(directory, entry.name)))
      .digest("hex");
  }
  return result;
}

export async function inspectCache<T>(
  directory: string,
  validate: (directory: string) => Promise<T>,
): Promise<CacheState<T>> {
  try {
    await fs.stat(directory);
  } catch (error) {
    if (isMissing(error)) return { state: "miss" };
    throw error;
  }
  try {
    // Existing caches predate completion manifests. They remain usable only
    // after the adapter validates every required artifact (never a paid miss).
    try {
      await fs.stat(path.join(directory, "failure.json"));
      throw new Error("previous provider attempt failed; recovery required");
    } catch (error) {
      if (!isMissing(error)) throw error;
    }
    let manifest: string | undefined;
    try {
      manifest = await fs.readFile(path.join(directory, manifestName), "utf8");
    } catch (error) {
      if (!isMissing(error)) throw error;
    }
    if (manifest !== undefined) {
      const parsed = JSON.parse(manifest);
      if (
        parsed.version !== 1 ||
        JSON.stringify(parsed.files) !==
          JSON.stringify(await inventory(directory))
      ) {
        throw new Error("cache completion manifest does not match artifacts");
      }
    }
    return { state: "complete", value: await validate(directory) };
  } catch (error) {
    return { state: "corrupt", error };
  }
}

/** Publish complete validated directories by rename. Corruption never triggers paid regeneration.
 * In-flight sharing is process-local; an exclusive sibling lease prevents duplicate work
 * by another process. A stale lease requires explicit operator recovery after inspection.
 */
export async function cachedArtifacts<T>(
  directory: string,
  validate: (directory: string) => Promise<T>,
  produce: (staging: string) => Promise<void>,
  options: CacheOptions = {},
): Promise<T> {
  directory = path.resolve(directory);
  const offline = options.offline ?? process.env.PIPELINE_OFFLINE === "1";
  // Offline callers inspect independently so they cannot join live remote work.
  if (!offline && pending.has(directory))
    return pending.get(directory) as Promise<T>;
  const run = async () => {
    const state = await inspectCache(directory, validate);
    if (state.state === "complete") return state.value;
    if (state.state === "corrupt")
      throw new Error(
        `corrupt/incomplete provider cache ${directory}; inspect and recover explicitly before retrying`,
        { cause: state.error },
      );
    if (offline) throw new Error(`offline provider cache miss: ${directory}`);
    await fs.mkdir(path.dirname(directory), { recursive: true });
    const lease = `${directory}.lock`;
    await fs.mkdir(lease); // EEXIST deliberately fails instead of issuing a duplicate paid request.
    let staging: string | undefined;
    try {
      const recheck = await inspectCache(directory, validate);
      if (recheck.state === "complete") return recheck.value;
      if (recheck.state === "corrupt")
        throw new Error(`cache changed during acquisition: ${directory}`, {
          cause: recheck.error,
        });
      staging = await fs.mkdtemp(`${directory}.partial-`);
      await produce(staging);
      await validate(staging);
      await fs.writeFile(
        path.join(staging, manifestName),
        JSON.stringify({ version: 1, files: await inventory(staging) }),
        { flag: "wx" },
      );
      await fs.rename(staging, directory);
      staging = undefined;
      return await validate(directory);
    } catch (error) {
      // Keep failed paid responses and block automatic regeneration on retry.
      if (staging) {
        await fs.writeFile(
          path.join(staging, "failure.json"),
          JSON.stringify({ message: String(error) }),
        );
        await fs.rename(staging, directory);
        staging = undefined;
      }
      throw error;
    } finally {
      await fs.rmdir(lease);
    }
  };
  const promise = run();
  if (!offline) pending.set(directory, promise);
  try {
    return await promise;
  } finally {
    if (pending.get(directory) === promise) pending.delete(directory);
  }
}

export async function validateGlb(file: string): Promise<void> {
  const bytes = await fs.readFile(file);
  if (
    bytes.length < 20 ||
    bytes.toString("ascii", 0, 4) !== "glTF" ||
    bytes.readUInt32LE(4) !== 2 ||
    bytes.readUInt32LE(8) !== bytes.length
  ) {
    throw new Error(`invalid/truncated GLB: ${file}`);
  }
  let offset = 12;
  let jsonSeen = false;
  while (offset < bytes.length) {
    if (offset + 8 > bytes.length)
      throw new Error(`truncated GLB chunk: ${file}`);
    const length = bytes.readUInt32LE(offset);
    const kind = bytes.readUInt32LE(offset + 4);
    if (length % 4 !== 0 || offset + 8 + length > bytes.length)
      throw new Error(`invalid GLB chunk length: ${file}`);
    if (offset === 12) {
      if (kind !== 0x4e4f534a)
        throw new Error(`GLB missing JSON chunk: ${file}`);
      const document = JSON.parse(
        bytes.toString("utf8", offset + 8, offset + 8 + length),
      );
      if (document.asset?.version !== "2.0")
        throw new Error(`invalid GLB asset: ${file}`);
      jsonSeen = true;
    }
    offset += 8 + length;
  }
  if (!jsonSeen) throw new Error(`GLB missing JSON: ${file}`);
}
