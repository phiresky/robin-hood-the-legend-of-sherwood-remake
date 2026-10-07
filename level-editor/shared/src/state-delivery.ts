import { safeLibraryPath } from "./projection-assets.ts";
import type { SceneAssetSource, Level3D, GameTransform } from "./level3d.ts";
import {
  validateNativeStatePresentation,
  type NativeStatePresentationContract,
} from "./native-state-presentation.ts";

export type PhysicalStateBinding = SceneAssetSource & {
  /** Mission placement of the reusable local origin, in glTF Y-up scene units.
   * Added to the loaded root translation; existing rotation, scale and local transforms survive.
   * Omission means zero translation for existing private preview contracts.
   */
  position?: [number, number, number];
};

/** An absent target is distinct from an unprepared or failed endpoint. */
export type PhysicalEndpoint = PhysicalStateBinding[] | { kind: "absent" };
export function physicalEndpointSources(endpoint: PhysicalEndpoint): PhysicalStateBinding[] {
  return Array.isArray(endpoint) ? endpoint : [];
}
/** Exact saved placement replaced only during the corresponding physical preview. */
export interface StaticStateReplacement {
  object_id: string;
  node: string;
  asset_id: string;
  model_sha256: string;
  transform: GameTransform;
  group_id?: string;
  group_transform?: GameTransform;
}

/** Exact artwork transitions and independently reviewed physical endpoints. */
export interface StateDeliveryContract {
  version: 1;
  scope: "controlled-state-preview";
  native: NativeStatePresentationContract;
  families: {
    id: string;
    element_ids: string[];
    background_ids: string[];
    patch_ids?: string[];
    hidden_initial_element_ids?: string[];
    /** Terminal entry of the last visible target; receiver completion retains its own clock. */
    body_terminal_tick: number;
    physical: { initial: PhysicalEndpoint; applied: PhysicalEndpoint };
    static_replacements?: { initial: StaticStateReplacement[]; applied: StaticStateReplacement[] };
  }[];
}
export function validateStateDelivery(value: unknown): asserts value is StateDeliveryContract {
  const fail = (message: string): never => {
    throw new Error(`State delivery: ${message}`);
  };
  if (!value || typeof value !== "object") fail("missing contract");
  const c = value as StateDeliveryContract;
  if (
    c.version !== 1 ||
    c.scope !== "controlled-state-preview" ||
    !Array.isArray(c.families) ||
    !c.families.length
  )
    fail("invalid contract");
  validateNativeStatePresentation(c.native);
  const families = new Set<string>(),
    members = new Set<string>();
  const hash = (v: unknown) => typeof v === "string" && /^[a-f0-9]{64}$/.test(v);
  for (const family of c.families) {
    if (
      !family ||
      typeof family.id !== "string" ||
      !family.id ||
      families.has(family.id) ||
      !Array.isArray(family.element_ids) ||
      (!family.element_ids.length && !family.patch_ids?.length) ||
      (family.patch_ids !== undefined && !Array.isArray(family.patch_ids)) ||
      (family.hidden_initial_element_ids !== undefined &&
        !Array.isArray(family.hidden_initial_element_ids)) ||
      !Array.isArray(family.background_ids) ||
      !Number.isSafeInteger(family.body_terminal_tick) ||
      family.body_terminal_tick < 0
    )
      fail("invalid family");
    families.add(family.id);
    let terminal = 0;
    for (const id of family.element_ids) {
      const e = c.native.elements.find((e) => e.id === id);
      if (
        !e ||
        e.source.kind !== "mission-target" ||
        e.active ||
        e.loop ||
        !e.frames.length ||
        members.has(id)
      )
        fail("invalid or shared transition element");
      members.add(id);
      terminal = Math.max(
        terminal,
        e!.frames.slice(0, -1).reduce((n, f) => n + f.delay + 1, 0),
      );
    }
    for (const id of family.hidden_initial_element_ids ?? []) {
      const e = c.native.elements.find((e) => e.id === id);
      if (
        !e ||
        e.source.kind !== "mission-target" ||
        e.active ||
        e.loop ||
        e.frames.length ||
        !e.initial_frame ||
        members.has(id)
      )
        fail("invalid hidden initial target");
      members.add(id);
    }
    for (const id of family.patch_ids ?? []) {
      const state = c.native.patch_states?.find((s) => s.id === id);
      if (!state || members.has(id)) fail("invalid or shared patch state");
      members.add(id);
      terminal = Math.max(
        terminal,
        Math.max(1, state!.transition.reduce((n, f) => n + f.delay + 1, 0) - 1),
      );
    }
    if (terminal !== family.body_terminal_tick)
      fail("body terminal timing differs from bound frames");
    for (const id of family.background_ids) {
      if (!c.native.background_states?.some((s) => s.id === id) || members.has(id))
        fail("invalid or shared background state");
      members.add(id);
    }
    if (!family.physical) fail("missing physical endpoints");
    if (family.static_replacements !== undefined) {
      const transform = (t: unknown): t is GameTransform => {
        if (!t || typeof t !== "object") return false;
        const value = t as GameTransform;
        return [value.dx, value.dy, value.dz, value.rot_deg].every(Number.isFinite);
      };
      for (const state of ["initial", "applied"] as const) {
        const rows = family.static_replacements[state];
        if (!Array.isArray(rows)) fail("invalid static replacements");
        const objects = new Set<string>();
        for (const row of rows) {
          if (
            !row ||
            ![row.object_id, row.node, row.asset_id].every(
              (v) => typeof v === "string" && v.length > 0,
            ) ||
            objects.has(row.object_id) ||
            !hash(row.model_sha256) ||
            !transform(row.transform) ||
            (row.group_id === undefined) !== (row.group_transform === undefined) ||
            (row.group_id !== undefined &&
              (typeof row.group_id !== "string" ||
                !row.group_id ||
                !transform(row.group_transform)))
          )
            fail("invalid static replacement");
          objects.add(row.object_id);
        }
      }
    }
    for (const endpoint of [family.physical.initial, family.physical.applied]) {
      if (
        endpoint &&
        !Array.isArray(endpoint) &&
        endpoint.kind === "absent" &&
        Object.keys(endpoint).length === 1
      )
        continue;
      if (!Array.isArray(endpoint) || !endpoint.length) fail("missing physical endpoint");
      const sources = endpoint as PhysicalStateBinding[];
      const sourceIds = new Set<string>();
      for (const source of sources) {
        if (
          !source ||
          typeof source.id !== "string" ||
          !source.id ||
          sourceIds.has(source.id) ||
          !["objects", "ground"].includes(source.role) ||
          !safeLibraryPath(source.model) ||
          !hash(source.model_sha256) ||
          !Array.isArray(source.resources) ||
          (source.position !== undefined &&
            (!Array.isArray(source.position) ||
              source.position.length !== 3 ||
              !source.position.every(Number.isFinite))) ||
          (source.model_scene !== undefined &&
            (typeof source.model_scene !== "string" || !source.model_scene))
        )
          fail("invalid physical source");
        sourceIds.add(source.id);
        for (const resource of source.resources)
          if (!resource || !safeLibraryPath(resource.path) || !hash(resource.sha256))
            fail("invalid physical resource");
      }
    }
  }
}

/** Reject edited or unrelated placements before any visible node is suppressed. */
export function verifyStaticStateReplacements(
  contract: StateDeliveryContract,
  document: Level3D,
): void {
  validateStateDelivery(contract);
  const sameTransform = (a: GameTransform, b: GameTransform) =>
    a.dx === b.dx && a.dy === b.dy && a.dz === b.dz && a.rot_deg === b.rot_deg;
  for (const family of contract.families)
    for (const rows of Object.values(family.static_replacements ?? {}))
      for (const row of rows) {
        const object = document.objects.find((o) => o.id === row.object_id);
        const asset = document.assetSources?.find((a) => a.id === row.asset_id);
        const group =
          row.group_id === undefined
            ? undefined
            : document.groups.find((g) => g.id === row.group_id);
        if (
          !object ||
          object.node !== row.node ||
          object.node.split(":")[1] !== row.asset_id ||
          !asset ||
          asset.model_sha256 !== row.model_sha256 ||
          !sameTransform(object.transform, row.transform) ||
          object.group !== row.group_id ||
          (row.group_id !== undefined &&
            (!group || !sameTransform(group.transform, row.group_transform!)))
        )
          throw new Error(
            `State replacement differs from the displayed placement: ${row.object_id}`,
          );
      }
}

/** Final patch animation keeps the existing native clock running after the transition. */
export function stateDeliveryLoopsAfterTransition(
  contract: StateDeliveryContract,
  familyId: string,
): boolean {
  const family = contract.families.find((f) => f.id === familyId);
  if (!family) throw new Error(`Unknown state family: ${familyId}`);
  return [
    ...(contract.native.patch_states ?? []).filter((s) => family.patch_ids?.includes(s.id)),
    ...(contract.native.background_states ?? []).filter((s) =>
      family.background_ids.includes(s.id),
    ),
  ].some((s) => s.final_loop && s.final.length > 0);
}

/** Source artwork loop preview; independent ambient elements retain their own clocks. */
export interface NativeLoopPreviewContract {
  version: 1;
  scope: "controlled-native-loop-preview";
  native: NativeStatePresentationContract;
  focus_element_id: string;
}
export function validateNativeLoopPreview(
  value: unknown,
): asserts value is NativeLoopPreviewContract {
  if (!value || typeof value !== "object") throw new Error("Missing native loop preview");
  const contract = value as NativeLoopPreviewContract;
  if (contract.version !== 1 || contract.scope !== "controlled-native-loop-preview")
    throw new Error("Invalid native loop preview");
  validateNativeStatePresentation(contract.native);
  const focus = contract.native.elements.find((e) => e.id === contract.focus_element_id);
  if (
    !focus ||
    !focus.active ||
    !focus.loop ||
    !focus.frames.length ||
    contract.native.background_states?.length ||
    contract.native.elements.some((e) => e.frames.length && !e.loop)
  )
    throw new Error("Native loop preview requires a visible looping focus and independent loops");
}
export function nativeLoopPreviewPeriod(contract: NativeLoopPreviewContract): number {
  validateNativeLoopPreview(contract);
  return contract.native.elements
    .find((e) => e.id === contract.focus_element_id)!
    .frames.reduce((ticks, frame) => ticks + frame.delay + 1, 0);
}

/** A controlled artwork preview does not imply physical endpoint coverage. */
export interface NativePatchPreviewContract {
  version: 1;
  kind: "native-patch";
  scope: string;
  native: NativeStatePresentationContract;
  focus_patch_id: string;
}
export function validateNativePatchPreview(
  value: unknown,
): asserts value is NativePatchPreviewContract {
  if (!value || typeof value !== "object") throw new Error("Missing native patch preview");
  const contract = value as NativePatchPreviewContract;
  if (
    contract.version !== 1 ||
    contract.kind !== "native-patch" ||
    typeof contract.scope !== "string" ||
    !contract.scope.trim() ||
    "physical" in contract ||
    "families" in contract
  )
    throw new Error("Invalid native patch preview");
  validateNativeStatePresentation(contract.native);
  const focus = contract.native.patch_states?.find((p) => p.id === contract.focus_patch_id);
  if (
    !focus ||
    !focus.transition.length ||
    (focus.integrate_in_background && focus.activation !== "phases") ||
    contract.native.background_states?.length ||
    contract.native.patch_states?.some(
      (p) => p.id !== focus.id && p.integrate_in_background && p.activation !== "initial-only",
    )
  )
    throw new Error("Native patch preview requires one controlled focus and initial context");
}
export function nativePatchPreviewTerminal(contract: NativePatchPreviewContract): number {
  validateNativePatchPreview(contract);
  const patch = contract.native.patch_states!.find((p) => p.id === contract.focus_patch_id)!;
  return Math.max(1, patch.transition.reduce((ticks, frame) => ticks + frame.delay + 1, 0) - 1);
}
export function nativePatchPreviewLoops(contract: NativePatchPreviewContract): boolean {
  validateNativePatchPreview(contract);
  const patch = contract.native.patch_states!.find((p) => p.id === contract.focus_patch_id)!;
  return patch.final_loop && patch.final.length > 0;
}
