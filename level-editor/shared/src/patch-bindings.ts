/** Mission patch IDs attached to one placed asset, keyed by model node name. */
export interface PatchBinding {
  hide?: string[];
  show?: string[];
  material?: { patch: string; state: "covered" | "revealed" };
}

export type PatchBindings = Record<string, PatchBinding>;

/** Asset-local appearance IDs mapped to mission patch IDs for a placement. */
export type AppearancePatches = Record<string, Record<string, string>>;

/** Descriptor-owned visibility augments, but cannot silently replace, mesh rules. */
export function assetPartAppearanceExtras(
  extras: Record<string, unknown>,
  appearance: Pick<PatchBinding, "hide" | "show"> | undefined,
): Record<string, unknown> {
  if (!appearance) return extras;
  const authored = patchBindingExtras(appearance);
  for (const [key, value] of Object.entries(authored))
    if (extras[key] !== undefined && JSON.stringify(extras[key]) !== JSON.stringify(value))
      throw new Error(`Asset part appearance conflicts with model ${key}`);
  return { ...extras, ...authored };
}

export function remapPatchExtras(extras: Record<string, unknown>, mapping: Record<string, string>) {
  const result = { ...extras };
  for (const field of ["reveal_hide_when_applied", "reveal_show_when_applied"] as const) {
    const value = result[field];
    if (Array.isArray(value)) result[field] = value.map((id: string) => mapping[id] ?? id);
  }
  if (typeof result.reveal_material_patch === "string")
    result.reveal_material_patch =
      mapping[result.reveal_material_patch] ?? result.reveal_material_patch;
  return result;
}

/** Extract only placement-specific display rules from publication node metadata. */
export function patchBindingsFromMetadata(
  metadata: Record<string, Record<string, unknown>> | undefined,
): PatchBindings | undefined {
  if (!metadata) return undefined;
  const bindings: PatchBindings = {};
  for (const [node, values] of Object.entries(metadata)) {
    const binding: PatchBinding = {};
    const hide = values.reveal_hide_when_applied;
    const show = values.reveal_show_when_applied;
    if (Array.isArray(hide) && hide.length) binding.hide = [...hide];
    if (Array.isArray(show) && show.length) binding.show = [...show];
    if (typeof values.reveal_material_patch === "string") {
      binding.material = {
        patch: values.reveal_material_patch,
        state: values.reveal_material_state as "covered" | "revealed",
      };
    }
    if (Object.keys(binding).length) bindings[node] = binding;
  }
  return Object.keys(bindings).length ? bindings : undefined;
}

export function patchBindingExtras(binding: PatchBinding): Record<string, unknown> {
  return {
    ...(binding.hide ? { reveal_hide_when_applied: binding.hide } : {}),
    ...(binding.show ? { reveal_show_when_applied: binding.show } : {}),
    ...(binding.material
      ? {
          reveal_material_patch: binding.material.patch,
          reveal_material_state: binding.material.state,
        }
      : {}),
  };
}

/** Derive one asset's local-to-mission mapping from reviewed per-node rules. */
export function deriveAppearancePatches(
  bindings: PatchBindings | undefined,
  nodes: ReadonlyMap<string, Record<string, unknown>>,
  mapping: Record<string, string>,
): void {
  for (const [name, binding] of Object.entries(bindings ?? {})) {
    const local = nodes.get(name);
    if (!local) throw new Error(`Patch binding node is missing from asset: ${name}`);
    const add = (source: string, target: string) => {
      if (mapping[source] !== undefined && mapping[source] !== target)
        throw new Error(`Conflicting mission patch for ${source}: ${mapping[source]} / ${target}`);
      mapping[source] = target;
    };
    for (const [field, key] of [
      ["hide", "reveal_hide_when_applied"],
      ["show", "reveal_show_when_applied"],
    ] as const) {
      const targets = binding[field];
      if (!targets) continue;
      const sources = local[key];
      if (Array.isArray(sources) && sources.length === targets.length)
        sources.forEach((source: string, index: number) => add(source, targets[index]!));
      else if (sources === undefined)
        throw new Error(`Patch rule lacks an asset-local trigger: ${name}`);
      else throw new Error(`Patch rule differs from asset-local triggers: ${name}`);
    }
    if (binding.material) {
      const source = local.reveal_material_patch;
      if (typeof source === "string" && local.reveal_material_state === binding.material.state)
        add(source, binding.material.patch);
      else if (source === undefined)
        throw new Error(`Patch material lacks an asset-local trigger: ${name}`);
      else throw new Error(`Patch material differs from asset-local state: ${name}`);
    }
  }
}

/** Compare every node, including nodes without explicit old bindings. */
export function assertPatchMappingEquivalent(
  bindings: PatchBindings | undefined,
  nodes: ReadonlyMap<string, Record<string, unknown>>,
  mapping: Record<string, string>,
  rootName?: string,
  rootRule?: PatchBinding,
) {
  const fields = [
    "reveal_hide_when_applied",
    "reveal_show_when_applied",
    "reveal_material_patch",
    "reveal_material_state",
  ] as const;
  for (const [name, extras] of nodes) {
    const before = { ...extras, ...patchBindingExtras(bindings?.[name] ?? {}) };
    const after = {
      ...remapPatchExtras(extras, mapping),
      ...(name === rootName && rootRule ? patchBindingExtras(rootRule) : {}),
    };
    for (const field of fields)
      if (JSON.stringify(before[field]) !== JSON.stringify(after[field]))
        throw new Error(`Patch mapping changes ${name}.${field}`);
  }
}

export function modelPartPatchNodes(
  model: {
    scene?: number;
    scenes: { nodes?: number[] }[];
    nodes: { name?: string; children?: number[]; extras?: Record<string, unknown> }[];
  },
  rootName: string,
): Map<string, Record<string, unknown>> {
  const roots = model.scenes[model.scene ?? 0]?.nodes ?? [];
  const reachable = new Set<number>();
  const walk = (index: number) => {
    if (reachable.has(index)) return;
    reachable.add(index);
    for (const child of model.nodes[index]?.children ?? []) walk(child);
  };
  for (const root of roots) walk(root);
  const part = [...reachable].find((index) => model.nodes[index]?.name === rootName);
  if (part === undefined) throw new Error(`Missing asset part node: ${rootName}`);
  const nodes = new Map<string, Record<string, unknown>>();
  const collect = (index: number) => {
    const node = model.nodes[index];
    if (!node?.name || nodes.has(node.name))
      throw new Error(`Missing or duplicate asset child node in ${rootName}`);
    nodes.set(node.name, node.extras ?? {});
    for (const child of node.children ?? []) collect(child);
  };
  collect(part);
  return nodes;
}

/** Shared descriptor parts remain visible; parts unique to one variant switch with the patch. */
export function endpointPatchRule(
  partNode: string,
  availableNodes: ReadonlySet<string>,
  patches: Readonly<AppearancePatches> | undefined,
): PatchBinding | undefined {
  for (const [asset, mapping] of Object.entries(patches ?? {})) {
    const patch = mapping.state;
    if (!patch) continue;
    const initial = `asset:${asset}:`;
    const applied = `asset:${asset}--state-applied:`;
    if (partNode.startsWith(initial)) {
      const node = partNode.slice(initial.length);
      if (!availableNodes.has(`${applied}${node}`)) return { hide: [patch] };
    }
    if (partNode.startsWith(applied)) {
      const node = partNode.slice(applied.length);
      if (!availableNodes.has(`${initial}${node}`)) return { show: [patch] };
    }
  }
  return undefined;
}
