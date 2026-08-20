// Scanning and loading of the hackable datadir (Data/Levels/*).
import type { Mission, MissionHeader, ProtoLevel } from "@rle/shared";
import { findFileCI, listDirs, listFiles, readImage, readJson, subdir } from "./fs";

export interface DatadirIndex {
  /** map name (rhp basename) -> ambiance dirs that contain its map PNG */
  maps: Map<string, string[]>;
  /** mission basename -> header (for grouping under maps) */
  missions: Map<string, MissionHeader>;
  levelsDir: FileSystemDirectoryHandle;
}

export async function scanDatadir(root: FileSystemDirectoryHandle): Promise<DatadirIndex> {
  const levelsDir = await subdir(root, ["Data", "Levels"]);
  if (!levelsDir) throw new Error("Not a hackable datadir: Data/Levels/ missing");

  const files = await listFiles(levelsDir);
  const mapNames = files
    .filter((f) => f.endsWith(".rhp.json"))
    .map((f) => f.slice(0, -".rhp.json".length));

  const maps = new Map<string, string[]>();
  const ambianceDirs = await listDirs(levelsDir);
  for (const name of mapNames) {
    const dirs: string[] = [];
    for (const amb of ambianceDirs) {
      const dir = await levelsDir.getDirectoryHandle(amb);
      if (await findFileCI(dir, `${name}.map.png`)) dirs.push(amb);
    }
    maps.set(name, dirs);
  }

  const missions = new Map<string, MissionHeader>();
  for (const f of files) {
    if (!f.endsWith(".rhm.json")) continue;
    const mission = await readJson<Mission>(levelsDir, f);
    missions.set(f.slice(0, -".rhm.json".length), mission.header);
  }

  return { maps, missions, levelsDir };
}

/** missions whose header.map_filename references this map (matches basename, case-insensitive) */
export function missionsForMap(index: DatadirIndex, mapName: string): string[] {
  const target = mapName.toLowerCase();
  const out: string[] = [];
  for (const [name, header] of index.missions) {
    const base = header.map_filename
      .replaceAll("\\", "/")
      .split("/")
      .pop()!
      .replace(/\.map$/i, "")
      .toLowerCase();
    if (base === target) out.push(name);
  }
  return out.sort();
}

export async function loadProtoLevel(
  index: DatadirIndex,
  mapName: string,
): Promise<ProtoLevel> {
  return readJson<ProtoLevel>(index.levelsDir, `${mapName}.rhp.json`);
}

export async function loadMission(index: DatadirIndex, missionName: string): Promise<Mission> {
  return readJson<Mission>(index.levelsDir, `${missionName}.rhm.json`);
}

export async function loadMapImage(
  index: DatadirIndex,
  mapName: string,
  ambiance: string,
): Promise<ImageBitmap> {
  const dir = await index.levelsDir.getDirectoryHandle(ambiance);
  const file = await findFileCI(dir, `${mapName}.map.png`);
  if (!file) throw new Error(`no ${mapName}.map.png in ${ambiance}/`);
  return readImage(dir, file);
}
