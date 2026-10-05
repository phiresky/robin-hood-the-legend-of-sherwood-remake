import { safeLibraryPath, type ProtoLevel } from "@rle/shared";
import { isNotFound, readJson, subdir } from "./fs.ts";
import {
  validateStateDelivery,
  validateNativeLoopPreview,
  physicalEndpointSources,
  type NativeLoopPreviewContract,
  type StateDeliveryContract,
} from "../../shared/src/state-delivery.ts";
import { verifyNativePresentationSource } from "./native-state-presentation.ts";
import type { MissionStateSource } from "./mission-state-layer.ts";

interface PinnedJson {
  path: string;
  sha256: string;
}
export interface MissionStateCatalogEntry {
  kind?: "transition" | "native-loop";
  id: string;
  name: string;
  map: string;
  mission: string;
  contract: PinnedJson;
  mission_data: PinnedJson;
  level_data: PinnedJson;
}
export function parseMissionStateCatalog(value: unknown): MissionStateCatalogEntry[] {
  const fail = (): never => {
    throw new Error("Invalid mission state catalog");
  };
  if (!value || typeof value !== "object") fail();
  const data = value as { version?: unknown; entries?: unknown };
  if (data.version !== 1 || !Array.isArray(data.entries)) fail();
  const ids = new Set<string>();
  const pin = (v: unknown) => {
    if (!v || typeof v !== "object") return false;
    const p = v as PinnedJson;
    return (
      typeof p.path === "string" &&
      safeLibraryPath(p.path) &&
      typeof p.sha256 === "string" &&
      /^[a-f0-9]{64}$/.test(p.sha256)
    );
  };
  return (data.entries as MissionStateCatalogEntry[]).map((entry) => {
    if (
      !entry ||
      (entry.kind !== undefined && !["transition", "native-loop"].includes(entry.kind)) ||
      ![entry.id, entry.name, entry.map, entry.mission].every(
        (v) => typeof v === "string" && v.trim().length > 0,
      ) ||
      ids.has(entry.id) ||
      !pin(entry.contract) ||
      !pin(entry.mission_data) ||
      !pin(entry.level_data)
    )
      fail();
    ids.add(entry.id);
    return structuredClone(entry);
  });
}
/** A missing optional catalog means no previews; a declared broken entry is an error. */
export async function loadMissionStateCatalog(
  root: FileSystemDirectoryHandle,
  map: string,
  mission: string,
) {
  const directory = await subdir(root, ["mission-states"]);
  if (!directory) return [];
  let value: unknown;
  try {
    value = await readJson(directory, "index.json");
  } catch (error) {
    if (isNotFound(error)) return [];
    throw error;
  }
  return parseMissionStateCatalog(value).filter(
    (entry) => entry.map === map && entry.mission === mission,
  );
}
async function pinnedJson(root: FileSystemDirectoryHandle, pin: PinnedJson): Promise<unknown> {
  const parts = pin.path.split("/"),
    name = parts.pop()!,
    directory = await subdir(root, parts);
  if (!directory) throw new Error(`Missing state resource: ${pin.path}`);
  const bytes = await (await (await directory.getFileHandle(name)).getFile()).arrayBuffer();
  const hash = Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)), (n) =>
    n.toString(16).padStart(2, "0"),
  ).join("");
  if (hash !== pin.sha256) throw new Error(`State resource changed: ${pin.path}`);
  try {
    return JSON.parse(new TextDecoder().decode(bytes));
  } catch (error) {
    throw new Error(`Invalid state JSON: ${pin.path}`, { cause: error });
  }
}
export async function loadMissionStatePreview(
  root: FileSystemDirectoryHandle,
  entry: MissionStateCatalogEntry,
): Promise<
  | { kind: "transition"; contract: StateDeliveryContract; source: MissionStateSource }
  | { kind: "native-loop"; contract: NativeLoopPreviewContract; source: MissionStateSource }
> {
  parseMissionStateCatalog({ version: 1, entries: [entry] });
  const [contract, data, level] = await Promise.all([
    pinnedJson(root, entry.contract),
    pinnedJson(root, entry.mission_data),
    pinnedJson(root, entry.level_data),
  ]);
  const loop = entry.kind === "native-loop";
  if (loop) validateNativeLoopPreview(contract);
  else validateStateDelivery(contract);
  const native = (contract as StateDeliveryContract | NativeLoopPreviewContract).native;
  if (native.mission !== entry.mission)
    throw new Error("State catalog mission differs from its contract");
  for (const family of loop ? [] : (contract as StateDeliveryContract).families)
    for (const binding of [
      ...physicalEndpointSources(family.physical.initial),
      ...physicalEndpointSources(family.physical.applied),
    ])
      if (!binding.position)
        throw new Error("Published state asset requires explicit local-origin placement");
  const source: MissionStateSource = {
    name: entry.mission,
    data: data as Record<string, unknown>,
    level: level as ProtoLevel,
    camera: { kind: "oblique-orthographic", elevation_deg: native.camera_elevation_deg },
  };
  await verifyNativePresentationSource(native, source);
  return loop
    ? { kind: "native-loop", contract: contract as NativeLoopPreviewContract, source }
    : { kind: "transition", contract: contract as StateDeliveryContract, source };
}
