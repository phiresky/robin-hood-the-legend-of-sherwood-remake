import { safeLibraryPath } from "./projection-assets.ts";
import type { SceneAssetSource } from "./level3d.ts";
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

/** Exact artwork transitions and independently reviewed physical endpoints. */
export interface StateDeliveryContract {
  version: 1;
  scope: "controlled-state-preview";
  native: NativeStatePresentationContract;
  families: {
    id: string;
    element_ids: string[];
    background_ids: string[];
    /** Terminal entry of the last visible target; receiver completion retains its own clock. */
    body_terminal_tick: number;
    physical: { initial: PhysicalStateBinding[]; applied: PhysicalStateBinding[] };
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
      !family.element_ids.length ||
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
    if (terminal !== family.body_terminal_tick)
      fail("body terminal timing differs from bound frames");
    for (const id of family.background_ids) {
      if (!c.native.background_states?.some((s) => s.id === id) || members.has(id))
        fail("invalid or shared background state");
      members.add(id);
    }
    if (!family.physical) fail("missing physical endpoints");
    for (const sources of [family.physical.initial, family.physical.applied]) {
      if (!Array.isArray(sources) || !sources.length) fail("missing physical endpoint");
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
