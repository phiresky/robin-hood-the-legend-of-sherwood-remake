import type { GameplayAssetDescriptor } from "./asset-gameplay.ts";
import type { Level3D } from "./level3d.ts";
import { gameplayStateAliases } from "./gameplay-state-views.ts";

/** Bind preview names through asset-local definitions, never through source-level indices. */
export function compileAppearanceBindings(
  document: Level3D,
  assets: ReadonlyMap<string, GameplayAssetDescriptor>,
  transitions: readonly { id: string; aliases?: string[] }[] = [],
  warn?: (message: string) => void,
): Map<string, Record<string, string>> {
  const result = new Map<string, Record<string, string>>();
  const aliases = new Map<
    string,
    { id: string; asset: string; appearance: string; join?: string }
  >();
  const canonical = new Map<string, string>();
  for (const transition of transitions) {
    for (const id of [transition.id, ...(transition.aliases ?? [])]) {
      if (canonical.has(id)) throw new Error(`Duplicate compiled transition identity: ${id}`);
      canonical.set(id, transition.id);
    }
  }
  const groups = new Map(document.groups.map((group) => [group.id, group]));
  const stateAliases = gameplayStateAliases(document, assets);
  for (const part of document.objects) {
    const group = part.group ? groups.get(part.group) : undefined;
    if (part.hidden || group?.hidden) continue;
    const view = /^asset:([^:]+):/.exec(part.node)?.[1];
    if (!view) continue;
    const asset = stateAliases.get(view) ?? view;
    const local = new Map<string, string>();
    const joins = new Map<string, string>();
    for (const transition of assets.get(asset)?.gameplay?.movementTransitions ?? []) {
      const placed = `${part.group ?? part.id}/${asset}/${transition.id}`;
      if (warn && !canonical.has(placed)) {
        for (const appearance of transition.appearances ?? [])
          warn(
            `Appearance ${appearance} on ${part.id}: transition omitted; exported in its initial visual state.`,
          );
        continue;
      }
      const id = canonical.get(placed) ?? placed;
      for (const appearance of transition.appearances ?? []) {
        if (local.has(appearance))
          throw new Error(`Multiply controlled asset appearance ${appearance}`);
        local.set(appearance, id);
        if (transition.join) joins.set(appearance, transition.join.key);
      }
    }
    const preview = part.group ? group?.patches?.[asset] : part.patches?.[asset];
    for (const [appearance, alias] of Object.entries(preview ?? {})) {
      const id = local.get(appearance);
      if (!id) {
        const message = `Missing asset gameplay binding for appearance ${appearance} on ${part.id}`;
        if (!warn) throw new Error(message);
        warn(`${message}; exported in its initial visual state.`);
        continue;
      }
      const previous = aliases.get(alias);
      const join = joins.get(appearance);
      if (
        previous !== undefined &&
        previous.id !== id &&
        (previous.asset !== asset || previous.appearance !== appearance) &&
        (!join || previous.join !== join)
      )
        throw new Error(`Shared appearance ${alias} needs an explicit joined gameplay transition`);
      aliases.set(alias, { id, asset, appearance, join });
    }
    const mapping = new Map<string, string>();
    for (const [appearance, id] of local) {
      const key = preview?.[appearance] ?? appearance;
      if (mapping.has(key) && mapping.get(key) !== id)
        throw new Error(`Ambiguous placed appearance ${key}`);
      mapping.set(key, id);
    }
    // Alternate models retain their local material names; endpoint rules use the
    // primary placement's preview name. Both refer to the same gameplay switch.
    if (view !== asset) {
      for (const [appearance, id] of local) {
        if (mapping.has(appearance) && mapping.get(appearance) !== id)
          throw new Error(`Ambiguous state-view appearance ${appearance}`);
        mapping.set(appearance, id);
      }
    }
    result.set(part.id, Object.fromEntries(mapping));
  }
  return result;
}

export interface NativeCompiledAppearanceBinding {
  source: { kind: "map-patch" | "mission-patch" | "map-animation"; index: number };
  /** Explicit retention is visible in the export receipt, never an implicit fallback. */
  delivery: { kind: "source-native" } | { kind: "compiled-transition"; transition_id: string };
}

/** Validate exhaustive source ownership before an engine-native export can be assembled. */
export function validateNativeAppearanceBindings(
  identities: readonly { kind: NativeCompiledAppearanceBinding["source"]["kind"]; index: number }[],
  bindings: readonly NativeCompiledAppearanceBinding[],
  transitions: readonly { id: string }[],
) {
  const key = (row: NativeCompiledAppearanceBinding["source"]) => `${row.kind}:${row.index}`;
  for (const row of identities)
    if (
      !["map-patch", "mission-patch", "map-animation"].includes(row.kind) ||
      !Number.isSafeInteger(row.index) ||
      row.index < 0
    )
      throw new Error("Invalid native source identity");
  const expected = new Set(identities.map(key));
  if (expected.size !== identities.length) throw new Error("Duplicate native source identity");
  const compiled = new Set(transitions.map((row) => row.id));
  if (compiled.size !== transitions.length)
    throw new Error("Duplicate compiled transition identity");
  const seen = new Set<string>(),
    owners = new Map<string, string>();
  for (const binding of bindings) {
    const source = key(binding.source);
    if (!expected.has(source) || seen.has(source))
      throw new Error(`Invalid native appearance owner: ${source}`);
    seen.add(source);
    if (binding.delivery.kind === "source-native") continue;
    if (binding.delivery.kind !== "compiled-transition" || binding.source.kind === "map-animation")
      throw new Error(`Unsupported native appearance delivery: ${source}`);
    const id = binding.delivery.transition_id;
    if (!compiled.has(id) || owners.has(id))
      throw new Error(`Missing or multiply controlled compiled transition: ${id}`);
    owners.set(id, source);
  }
  if (seen.size !== expected.size)
    throw new Error(
      `Incomplete native appearance bindings: ${[...expected].filter((id) => !seen.has(id)).join(", ")}`,
    );
  if (owners.size !== compiled.size)
    throw new Error("Compiled transition has no authoritative native patch owner");
  return new Map(owners);
}
