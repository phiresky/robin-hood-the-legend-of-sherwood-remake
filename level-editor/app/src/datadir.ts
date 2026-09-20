// Scanning and loading of the hackable datadir (Data/Levels/*).
import type { ProtoLevel } from "@rle/shared";
import { parseProtoLevel } from "@rle/shared";
import { listFiles, readJson, subdir } from "./fs.ts";

export interface DatadirIndex {
  /** Original-case map basenames; level data is validated only when opened. */
  maps: Set<string>;
  missions?: string[];
  root?: FileSystemDirectoryHandle;
  levelsDir: FileSystemDirectoryHandle;
}

export async function scanDatadir(
  root: FileSystemDirectoryHandle,
): Promise<DatadirIndex> {
  const levelsDir = await subdir(root, ["Data", "Levels"]);
  if (!levelsDir)
    throw new Error("Not a hackable datadir: Data/Levels/ missing");

  const files = await listFiles(levelsDir);
  const mapNames = files
    .filter((f) => f.endsWith(".rhp.json"))
    .map((f) => f.slice(0, -".rhp.json".length));

  return { maps: new Set(mapNames), levelsDir, root, missions: files
    .filter((f) => f.endsWith(".rhm.json"))
    .map((f) => f.slice(0, -".rhm.json".length)).sort() };
}

export async function loadProtoLevel(
  index: DatadirIndex,
  mapName: string,
): Promise<ProtoLevel> {
  return parseProtoLevel(
    await readJson<unknown>(index.levelsDir, `${mapName}.rhp.json`),
  );
}
