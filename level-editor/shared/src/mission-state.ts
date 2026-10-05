import type { SceneAssetSource } from "./level3d.ts";
import type { Vec3 } from "./scene.ts";
import { safeLibraryPath } from "./projection-assets.ts";

export type MissionStateTiming =
  | { mode: "loop"; cycleTicks: number }
  | { mode: "clamp"; terminalTick: number };

/** Preview bindings, separate from editable mission serialization and script execution. */
export interface MissionStateContract {
  version: 1;
  mission: string;
  /** SHA256 of UTF8 JSON.stringify(parsed mission/level data), preserving parsed field order. */
  mission_data_sha256: string;
  level_data_sha256: string;
  camera_elevation_deg: number;
  targets: {
    id: string;
    target_index: number;
    /** Hash of JSON.stringify(the exact indexed source target). */
    target_sha256: string;
    source: SceneAssetSource;
    /** glTF Y-up coordinate of the native physical anchor in the exported model. */
    model_origin: Vec3;
    representation: "physical" | "native-appearance";
    actions: { action: number; clip: string; timing: MissionStateTiming }[];
  }[];
}

export function validateMissionStateContract(
  value: unknown,
): asserts value is MissionStateContract {
  const fail = (detail: string): never => {
    throw new Error(`Mission state: ${detail}`);
  };
  const hash = (v: unknown) => typeof v === "string" && /^[a-f0-9]{64}$/.test(v);
  const uint = (v: unknown) => typeof v === "number" && Number.isSafeInteger(v) && v >= 0;
  const text = (v: unknown) => typeof v === "string" && v.trim().length > 0;
  if (!value || typeof value !== "object") fail("expected contract");
  const c = value as MissionStateContract;
  if (
    c.version !== 1 ||
    !text(c.mission) ||
    !hash(c.mission_data_sha256) ||
    !hash(c.level_data_sha256) ||
    !Number.isFinite(c.camera_elevation_deg) ||
    c.camera_elevation_deg <= 0 ||
    c.camera_elevation_deg >= 90 ||
    !Array.isArray(c.targets)
  )
    fail("invalid source binding");
  const ids = new Set<string>(),
    indices = new Set<number>();
  for (const t of c.targets) {
    if (
      !t ||
      !text(t.id) ||
      ids.has(t.id) ||
      !uint(t.target_index) ||
      indices.has(t.target_index) ||
      !hash(t.target_sha256)
    )
      fail("invalid or duplicate target");
    ids.add(t.id);
    indices.add(t.target_index);
    if (
      !Array.isArray(t.model_origin) ||
      t.model_origin.length !== 3 ||
      !t.model_origin.every(Number.isFinite) ||
      !["physical", "native-appearance"].includes(t.representation)
    )
      fail("invalid placement representation");
    const s = t.source;
    if (
      !s ||
      !text(s.id) ||
      s.role !== "objects" ||
      !safeLibraryPath(s.model) ||
      !hash(s.model_sha256) ||
      !Array.isArray(s.resources) ||
      (s.model_scene !== undefined && !text(s.model_scene))
    )
      fail("invalid model binding");
    for (const r of s.resources)
      if (!r || !safeLibraryPath(r.path) || !hash(r.sha256)) fail("invalid resource binding");
    if (!Array.isArray(t.actions) || !t.actions.length) fail("missing actions");
    const actions = new Set<number>();
    for (const a of t.actions) {
      if (!a || !uint(a.action) || actions.has(a.action) || !text(a.clip) || !a.timing)
        fail("invalid action");
      actions.add(a.action);
      if (a.timing.mode === "loop") {
        if (!uint(a.timing.cycleTicks) || a.timing.cycleTicks === 0) fail("invalid cycle");
      } else if (a.timing.mode === "clamp") {
        if (!uint(a.timing.terminalTick)) fail("invalid terminal tick");
      } else fail("unknown timing mode");
    }
  }
}
