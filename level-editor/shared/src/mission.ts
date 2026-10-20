import type { Vec3 } from "./scene.ts";

export interface MissionSpawnPoint {
  id: string;
  name: string;
  position: Vec3;
  direction: number;
  /** Omitted for a slot filled by the selected campaign team. */
  profile?: number;
}
export interface MissionSoldier {
  id: string;
  name: string;
  position: Vec3;
  direction: number;
  profile: string;
  allegiance: number;
}
/** Explicit mission authoring, independent of map assets. */
export interface EditorMission {
  version: 1;
  spawnPoints: MissionSpawnPoint[];
  soldiers: MissionSoldier[];
  importedFrom?: string;
  importWarnings?: string[];
}

export function validateMission(value: unknown): asserts value is EditorMission {
  const fail = (): never => {
    throw new Error("Mission: invalid spawn point or soldier definition");
  };
  if (!value || typeof value !== "object") fail();
  const mission = value as EditorMission;
  if (
    mission.version !== 1 ||
    !Array.isArray(mission.spawnPoints) ||
    !Array.isArray(mission.soldiers)
  )
    fail();
  if (mission.spawnPoints.length > 65535) fail();
  if (mission.importedFrom !== undefined && typeof mission.importedFrom !== "string") fail();
  if (
    mission.importWarnings !== undefined &&
    (!Array.isArray(mission.importWarnings) ||
      !mission.importWarnings.every((warning) => typeof warning === "string"))
  )
    fail();
  const ids = new Set<string>();
  for (const actor of [...mission.spawnPoints, ...mission.soldiers]) {
    if (
      !actor ||
      typeof actor.id !== "string" ||
      !actor.id ||
      ids.has(actor.id) ||
      typeof actor.name !== "string" ||
      !Array.isArray(actor.position) ||
      actor.position.length !== 3 ||
      !actor.position.every(Number.isFinite) ||
      !Number.isInteger(actor.direction) ||
      actor.direction < 0 ||
      actor.direction > 15
    )
      fail();
    ids.add(actor.id);
  }
  for (const spawn of mission.spawnPoints)
    if (
      spawn.profile !== undefined &&
      (!Number.isInteger(spawn.profile) || spawn.profile < 0 || spawn.profile > 0xffffffff)
    )
      fail();
  for (const soldier of mission.soldiers)
    if (
      typeof soldier.profile !== "string" ||
      !soldier.profile.trim() ||
      !Number.isInteger(soldier.allegiance) ||
      soldier.allegiance < 0 ||
      soldier.allegiance > 65535
    )
      fail();
}
