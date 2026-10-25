import type {
  RuntimeStateObservation,
  RuntimeSpriteObservation,
  StateRuntimeBinding,
} from "./state-runtime-adapter.ts";

/** Source identity comes from the verified mission load, not the displayed map name. */
export interface RuntimeCaptureIdentity {
  mission: string;
  mission_data_sha256: string;
  level_data_sha256: string;
}
export interface RuntimeCaptureManifest extends RuntimeCaptureIdentity {
  engine_sha256: string;
  assets_sha256: string;
}
type ObjectValue = Record<string, unknown>;
function object(value: unknown, label: string): ObjectValue {
  if (!value || typeof value !== "object" || Array.isArray(value))
    throw new Error(`Missing ${label}`);
  return value as ObjectValue;
}
function array(value: unknown, label: string): unknown[] {
  if (!Array.isArray(value)) throw new Error(`Missing ${label}`);
  return value;
}
function integer(value: unknown, label: string, max = Number.MAX_SAFE_INTEGER): number {
  if (!Number.isSafeInteger(value) || (value as number) < 0 || (value as number) > max)
    throw new Error(`Invalid ${label}`);
  return value as number;
}
function boolean(value: unknown, label: string): boolean {
  if (typeof value !== "boolean") throw new Error(`Invalid ${label}`);
  return value;
}
function digest(value: string): boolean {
  return /^[a-f0-9]{64}$/.test(value);
}

/** Decode a typed actor handle without treating a patch index as an entity slot. */
export function runtimeActorSlot(handle: unknown): number {
  const value = integer(handle, "actor handle", 0x7fffffff);
  if ((value & 0xf0000000) !== 0x10000000) throw new Error("Invalid actor handle kind");
  return value & 0x0fffffff;
}
function spriteFromEntity(value: unknown, kind: "Fx" | "Target"): RuntimeSpriteObservation {
  const entity = object(value, "entity slot");
  if (Object.keys(entity).length !== 1 || !(kind in entity))
    throw new Error(`Expected ${kind} entity`);
  const element = object(object(entity[kind], kind).element, "element");
  const sprite = object(element.sprite, "sprite");
  if (typeof sprite.last_action !== "string" || !sprite.last_action)
    throw new Error("Missing sprite action");
  return {
    active: boolean(element.active, "entity activity"),
    row: integer(sprite.current_row, "sprite row", 65535),
    frame: integer(sprite.current_frame, "sprite frame", 65535),
    count: integer(sprite.frame_count, "sprite counter", 65535),
    last_action: sprite.last_action,
    ...(kind === "Target" && object(entity[kind], kind).target !== undefined
      ? {
          target_progression: integer(
            object(object(entity[kind], kind).target, "target").progression,
            "target progression",
          ),
        }
      : {}),
  };
}
async function sha256(bytes: Uint8Array): Promise<string> {
  const hash = await crypto.subtle.digest("SHA-256", new Uint8Array(bytes));
  return Array.from(new Uint8Array(hash), (b) => b.toString(16).padStart(2, "0")).join("");
}

/**
 * Decode only the current domain-based snapshot schema after exact payload checks.
 * Manifest source identity must be supplied by the verified launch/capture owner.
 * This does not establish a mission identity from a map name or from sprite rows.
 */
export async function decodeRuntimeCapture(
  engineBytes: Uint8Array,
  assetsBytes: Uint8Array,
  manifest: RuntimeCaptureManifest,
  expected: RuntimeCaptureIdentity,
  bindings: readonly StateRuntimeBinding[],
): Promise<RuntimeStateObservation> {
  // Freeze caller-owned bytes and metadata before the asynchronous hash boundary.
  const engineCopy = new Uint8Array(engineBytes),
    assetsCopy = new Uint8Array(assetsBytes);
  const identity = structuredClone(manifest),
    source = structuredClone(expected),
    prepared = structuredClone(bindings);
  if (
    !source.mission ||
    !digest(source.mission_data_sha256) ||
    !digest(source.level_data_sha256) ||
    identity.mission !== source.mission ||
    identity.mission_data_sha256 !== source.mission_data_sha256 ||
    identity.level_data_sha256 !== source.level_data_sha256 ||
    !digest(identity.engine_sha256) ||
    !digest(identity.assets_sha256)
  )
    throw new Error("Capture source identity differs");
  const [engineHash, assetsHash] = await Promise.all([sha256(engineCopy), sha256(assetsCopy)]);
  if (engineHash !== identity.engine_sha256 || assetsHash !== identity.assets_sha256)
    throw new Error("Capture payload hash differs");
  const engine = object(
    JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(engineCopy)),
    "engine snapshot",
  );
  const assets = object(
    JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(assetsCopy)),
    "level assets",
  );
  const frame = integer(object(engine.control, "control").frame_counter, "engine frame");
  const entities = array(object(engine.world, "world").entities, "entity slots");
  const patches = array(
    object(object(engine.script_domains, "script domains").interactables, "interactables").patches,
    "patches",
  );
  const handles = array(
    object(assets.assets, "assets").patch_entity_handles,
    "patch entity handles",
  );
  const owners = new Map<number, RuntimeStateObservation["owners"][number]>();
  const observations = new Map<number, RuntimeStateObservation["patches"][number]>();
  const keys = new Set<string>();
  for (const binding of prepared) {
    const key = `${binding.kind}:${binding.id}`;
    if (keys.has(key)) throw new Error("Duplicate runtime binding");
    keys.add(key);
    const index = integer(binding.runtime_index, "runtime index");
    if (binding.kind === "element") {
      owners.set(index, { index, sprite: spriteFromEntity(entities[index], "Target") });
      continue;
    }
    if (binding.kind !== "patch" && binding.kind !== "background")
      throw new Error("Invalid runtime binding kind");
    const patch = object(patches[index], "bound patch");
    if (index >= handles.length) throw new Error("Missing patch entity handle entry");
    const handle = handles[index];
    // Explicit null means the loaded patch has no FX. Missing/invalid slots are errors.
    const sprite =
      handle === null ? null : spriteFromEntity(entities[runtimeActorSlot(handle)], "Fx");
    observations.set(index, {
      index,
      applied: boolean(patch.applied, "patch applied"),
      in_transition: boolean(patch.in_transition, "patch transition"),
      sprite,
    });
  }
  return {
    mission: source.mission,
    engine_frame: frame,
    owners: [...owners.values()],
    patches: [...observations.values()],
  };
}
