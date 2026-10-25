import {
  validateSourceCoverVisibility,
  type SourceCoverVisibility,
} from "./source-cover-visibility.ts";
import { validateReviewedNodePoses, type ReviewedNodePose } from "./reviewed-node-poses.ts";
import { validateMissionStateContract, type MissionStateContract } from "./mission-state.ts";
import { safeLibraryPath } from "./projection-assets.ts";
import type { SceneAssetSource, Level3D, GameTransform } from "./level3d.ts";
import {
  validateNativeStatePresentation,
  nativePresentationFrame,
  type NativeStatePresentationContract,
  type NativeImageResource,
  type NativeInitialContextMode,
  type NativePresentationFrame,
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

/** Reviewed Y-up offsets sampled at the bound patch's exact frame boundaries.
 * The applied model already contains its terminal pose and receives no offset.
 */
export interface PhysicalPatchTranslation {
  kind: "native-patch-translation";
  patch_id: string;
  source_motion_sha256: string;
  translation_frames: [number, number, number][];
}

/** A mechanism appears at activation and keeps its baked terminal pose after completion. */
export interface PhysicalPatchNodePoses {
  kind: "native-patch-node-poses";
  patch_id: string;
  source_motion_sha256: string;
  node_frames: ReviewedNodePose[][];
}

/** Reviewed node poses driven by one target's action frames, independently of receiver patches. */
export interface PhysicalElementNodePoses {
  kind: "native-element-node-poses";
  element_id: string;
  source_motion_sha256: string;
  node_frames: ReviewedNodePose[][];
}

/** Native target frames until a reviewed start frame, then one pinned glTF clip on the target's own clock.
 * The clip is sampled at the target's integer source tick without resampling. Its keys end at the
 * family terminal tick, and its terminal pose must reproduce the separately pinned applied model.
 */
export interface PhysicalElementClip {
  kind: "native-element-clip";
  element_id: string;
  source_motion_sha256: string;
  /** First source frame drawn by the clip; every earlier frame keeps the native target artwork. */
  start_frame: number;
  binding: PhysicalStateBinding;
  clip: string;
  /** Ticks from the first tick of start_frame to body_terminal_tick. */
  duration_ticks: number;
}

/** First source tick of a target frame: the summed frame durations before it. */
export function nativeElementFrameStartTick(frames: readonly { delay: number }[], frame: number) {
  return frames.slice(0, frame).reduce((ticks, f) => ticks + f.delay + 1, 0);
}

/** Compiled placed movement-transition ids are `<placement id>/<transition id>`. */
const GAMEPLAY_TRANSITION_ID = /^[^/]+(\/[^/]+)+$/;

/** One retained assembly handed between independently clocked source target stages. */
export interface PhysicalElementStagePoses {
  kind: "native-element-stage-poses";
  source_motion_sha256: string;
  stages: {
    element_id: string;
    /** Null is an explicitly reviewed transparent source frame, not a missing pose. */
    node_frames: (ReviewedNodePose[] | null)[];
  }[];
}

/** Reviewed discrete models retain the last transition model as their applied state. */
export interface PhysicalPatchModelFrames {
  kind: "native-patch-model-frames";
  patch_id: string;
  source_motion_sha256: string;
  model_frames: PhysicalStateBinding[][];
}

export interface PhysicalMaterialFrame {
  duration_ticks: number;
  materials: {
    node: string;
    slot: number;
    material_index: number;
    material_name: string;
    texture_index: number;
    tex_coord: number;
    alpha_mode: "OPAQUE" | "MASK" | "BLEND";
  }[];
}

/** A patch phase binds one hash-pinned model and a named glTF clip. */
export interface PhysicalPatchClip {
  binding: PhysicalStateBinding;
  clip: string;
  duration_ticks: number;
  material_frames?: PhysicalMaterialFrame[];
}
export interface PhysicalPatchClips {
  /** Exact initial source frames; decoded alpha must be zero before physical loading. */
  transparent_initial_frames?: NativePresentationFrame[];
  kind: "native-patch-clips";
  patch_id: string;
  source_motion_sha256: string;
  transition: PhysicalPatchClip;
  final: PhysicalPatchClip | { kind: "absent" };
}

/** Pixel values of a reviewed source-domain mask, one 8-bit grey PNG per source frame. */
export const NATIVE_REMAINDER_MASK_VALUES = {
  transparent: 0,
  physical: 1,
  native: 2,
  ambiguous: 3,
} as const;

/** One reviewed source frame split between kept native pixels and a physical model. */
export interface NativeRemainderFrame {
  mask: NativeImageResource;
  /** The physical representation drawn with the kept pixels at the same source tick. */
  physical: "applied";
}

/** Kept native pixels of one target track, drawn with the family's physical model.
 * Pixels whose mask value is not listed in drawn_values are cleared after frame selection.
 */
export interface NativeRemainderBinding {
  element_id: string;
  encoding: "source-domain-v1";
  /** Must keep native-owned pixels; ambiguous pixels are kept only when listed. */
  drawn_values: (2 | 3)[];
  /** Kept native pixels win every overlap with the physical model. */
  priority: "remainder-over-physical";
  /** One entry per source frame. Null keeps the whole native frame and draws no physical model. */
  frames: (NativeRemainderFrame | null)[];
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
    /** Compiled placed movement transitions that switch with this family: the initial endpoint is
     * the not-applied transition state and the applied endpoint its applied state. Verified
     * against a compiled map (verifyStateGameplayLinks), never inferred.
     */
    gameplay_transition_ids?: string[];
    /** Terminal entry of the last visible target; receiver completion retains its own clock. */
    body_terminal_tick: number;
    physical: {
      initial: PhysicalEndpoint;
      applied: PhysicalEndpoint;
      motion?:
        | PhysicalPatchClips
        | PhysicalPatchTranslation
        | PhysicalPatchNodePoses
        | PhysicalPatchModelFrames
        | PhysicalElementNodePoses
        | PhysicalElementStagePoses
        | PhysicalElementClip;
    };
    static_replacements?: { initial: StaticStateReplacement[]; applied: StaticStateReplacement[] };
    native_remainder?: NativeRemainderBinding[];
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
    members = new Set<string>(),
    links = new Set<string>();
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
    if (family.gameplay_transition_ids !== undefined) {
      const ids = family.gameplay_transition_ids;
      if (
        !Array.isArray(ids) ||
        !ids.length ||
        ids.some((id) => typeof id !== "string" || !GAMEPLAY_TRANSITION_ID.test(id)) ||
        new Set(ids).size !== ids.length
      )
        fail("invalid gameplay transition links");
      if (!family.background_ids.length && !family.patch_ids?.length)
        fail("gameplay transition links require a bound background or patch state");
      for (const id of ids) {
        if (links.has(id)) fail("gameplay transition linked by two families");
        links.add(id);
      }
    }
    if (!family.physical) fail("missing physical endpoints");
    const motion = family.physical.motion;
    if (motion?.kind === "native-element-clip") validateElementClip(c, family, motion);
    else if (motion?.kind === "native-patch-clips") {
      const patch = c.native.patch_states?.find((p) => p.id === motion.patch_id);
      if (
        !hash(motion.source_motion_sha256) ||
        !patch?.definitive ||
        !family.patch_ids?.includes(motion.patch_id) ||
        family.element_ids.length ||
        !patch.transition.length
      )
        fail("invalid patch clip source binding");
      const initialProof = motion.transparent_initial_frames;
      const frameIdentity = (frame: NativePresentationFrame) =>
        JSON.stringify([
          frame.path,
          frame.sha256,
          frame.width,
          frame.height,
          frame.offset,
          frame.delay,
          frame.shadow_key?.rgb,
          frame.shadow_key?.strength_percent,
          frame.shadow_key?.pixel_format,
        ]);
      if (patch!.initial.length) {
        if (
          !Array.isArray(initialProof) ||
          initialProof.length !== patch!.initial.length ||
          initialProof.some(
            (frame, index) =>
              !frame || frameIdentity(frame) !== frameIdentity(patch!.initial[index]!),
          )
        )
          fail("transparent initial frame proof differs from bound patch");
      } else if (initialProof !== undefined) {
        fail("transparent initial frame proof requires bound initial frames");
      }
      const validateClip = (phase: PhysicalPatchClip, source: { delay: number }[]) => {
        const duration = source.reduce((n, frame) => n + frame.delay + 1, 0);
        if (
          !phase ||
          typeof phase.clip !== "string" ||
          !phase.clip.trim() ||
          !Number.isSafeInteger(phase.duration_ticks) ||
          phase.duration_ticks !== duration
        )
          fail("patch clip timing differs from source phase");
        if (phase.material_frames !== undefined) {
          if (
            !Array.isArray(phase.material_frames) ||
            phase.material_frames.length !== source.length
          )
            fail("material frame count differs from source phase");
          let targets: string | undefined;
          for (const [i, frame] of phase.material_frames.entries()) {
            if (
              frame.duration_ticks !== source[i]!.delay + 1 ||
              !Array.isArray(frame.materials) ||
              !frame.materials.length
            )
              fail("material frame timing or bindings differ from source phase");
            const keys = frame.materials
              .map((m) => {
                if (
                  !m.node ||
                  !m.material_name ||
                  ![m.slot, m.material_index, m.texture_index, m.tex_coord].every(
                    (n) => Number.isSafeInteger(n) && n >= 0,
                  ) ||
                  !["OPAQUE", "MASK", "BLEND"].includes(m.alpha_mode)
                )
                  fail("invalid material frame binding");
                return JSON.stringify([m.node, m.slot]);
              })
              .sort();
            if (
              new Set(keys).size !== keys.length ||
              (targets !== undefined && targets !== JSON.stringify(keys))
            )
              fail("material frame targets differ or duplicate");
            targets = JSON.stringify(keys);
          }
        }
      };
      validateClip(motion.transition, patch!.transition);
      if (!motion.final || typeof motion.final !== "object") fail("missing final clip phase");
      if ("kind" in motion.final) {
        if (
          motion.final.kind !== "absent" ||
          Object.keys(motion.final).length !== 1 ||
          patch!.final.length ||
          Array.isArray(family.physical.applied)
        )
          fail("invalid absent final clip phase");
      } else {
        if (!patch!.final.length || !patch!.final_loop)
          fail("patch clips require a looping final phase or explicit absence");
        validateClip(motion.final, patch!.final);
        if (JSON.stringify(family.physical.applied) !== JSON.stringify([motion.final.binding]))
          fail("final clip differs from applied model binding");
      }
    } else if (motion?.kind === "native-element-stage-poses") {
      if (
        !hash(motion.source_motion_sha256) ||
        !Array.isArray(motion.stages) ||
        motion.stages.length < 2 ||
        family.patch_ids?.length ||
        family.hidden_initial_element_ids?.length ||
        !Array.isArray(family.physical.applied) ||
        family.physical.applied.length !== 1 ||
        motion.stages.length !== family.element_ids.length ||
        new Set(motion.stages.map((stage) => stage.element_id)).size !== motion.stages.length
      )
        fail("invalid reviewed staged target binding");
      for (const stage of motion.stages) {
        const element = c.native.elements.find((e) => e.id === stage.element_id);
        if (
          !family.element_ids.includes(stage.element_id) ||
          !element ||
          element.loop ||
          !Array.isArray(stage.node_frames) ||
          stage.node_frames.length !== element.frames.length ||
          !stage.node_frames.some((frame) => frame !== null)
        )
          fail("invalid reviewed stage source frames");
      }
      const rows = motion.stages.flatMap((stage) =>
        stage.node_frames.filter((frame) => frame !== null),
      );
      validateReviewedNodePoses(rows);
      const last = motion.stages.at(-1)!.node_frames.at(-1);
      if (!last || last.some(({ matrix }) => matrix.some((v, i) => v !== (i % 5 === 0 ? 1 : 0))))
        fail("terminal stage differs from baked applied model");
    } else if (motion?.kind === "native-element-node-poses") {
      const element = c.native.elements.find((e) => e.id === motion.element_id);
      if (
        !hash(motion.source_motion_sha256) ||
        family.element_ids.length !== 1 ||
        family.element_ids[0] !== motion.element_id ||
        family.patch_ids?.length ||
        family.hidden_initial_element_ids?.length ||
        !element?.frames.length ||
        element.loop ||
        !Array.isArray(family.physical.applied) ||
        family.physical.applied.length !== 1 ||
        !Array.isArray(motion.node_frames) ||
        motion.node_frames.length !== element.frames.length
      )
        fail("invalid reviewed target motion binding");
      validateReviewedNodePoses(motion.node_frames);
      if (
        motion.node_frames
          .at(-1)!
          .some(({ matrix }) => matrix.some((v, i) => v !== (i % 5 === 0 ? 1 : 0)))
      )
        fail("terminal node pose differs from baked applied model");
    } else if (motion !== undefined) {
      const patch = c.native.patch_states?.find((p) => p.id === motion?.patch_id);
      if (
        !motion ||
        !hash(motion.source_motion_sha256) ||
        family.patch_ids?.length !== 1 ||
        family.patch_ids[0] !== motion.patch_id ||
        family.element_ids.length ||
        family.hidden_initial_element_ids?.length ||
        !patch?.definitive ||
        !patch.transition.length ||
        !Array.isArray(family.physical.applied) ||
        (motion.kind !== "native-patch-model-frames" && family.physical.applied.length !== 1)
      )
        fail("invalid reviewed patch motion binding");
      if (motion.kind === "native-patch-translation") {
        if (
          !Array.isArray(motion.translation_frames) ||
          motion.translation_frames.length !== patch!.transition.length ||
          motion.translation_frames.some(
            (v) => !Array.isArray(v) || v.length !== 3 || !v.every(Number.isFinite),
          ) ||
          motion.translation_frames[0]?.some((v) => v !== 0) ||
          !Array.isArray(family.physical.initial) ||
          family.physical.initial.length !== 1
        )
          fail("invalid reviewed patch translation");
      } else if (motion.kind === "native-patch-node-poses") {
        if (
          Array.isArray(family.physical.initial) ||
          family.physical.initial?.kind !== "absent" ||
          !Array.isArray(motion.node_frames) ||
          motion.node_frames.length !== patch!.transition.length
        )
          fail("invalid reviewed appearing mechanism");
        validateReviewedNodePoses(motion.node_frames);
        if (
          motion.node_frames
            .at(-1)!
            .some(({ matrix }) => matrix.some((v, i) => v !== (i % 5 === 0 ? 1 : 0)))
        )
          fail("terminal node pose differs from baked applied model");
      } else if (motion.kind === "native-patch-model-frames") {
        if (patch!.final.length) fail("discrete model frames do not support a final animation");
        if (
          !Array.isArray(motion.model_frames) ||
          motion.model_frames.length !== patch!.transition.length ||
          motion.model_frames.some((frame) => !Array.isArray(frame) || !frame.length)
        )
          fail("invalid reviewed patch model frames");
      } else fail("unknown reviewed physical motion");
    }
    if (family.native_remainder !== undefined) validateNativeRemainder(c, family);
    if (family.static_replacements !== undefined) {
      for (const rows of Object.values(family.static_replacements))
        validateStaticReplacementRows(rows);
      if (
        !Array.isArray(family.static_replacements.initial) ||
        !Array.isArray(family.static_replacements.applied)
      )
        fail("invalid static replacements");
    }
    for (const endpoint of physicalFamilyEndpoints(family)) {
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
    if (motion?.kind === "native-patch-model-frames") {
      const signature = (bindings: PhysicalStateBinding[]) =>
        JSON.stringify(
          bindings.map((s) => [
            s.id,
            s.role,
            s.model,
            s.model_sha256,
            s.model_scene,
            s.descriptor,
            s.descriptor_sha256,
            s.position,
            s.resources.map((r) => [r.path, r.sha256]),
          ]),
        );
      if (
        signature(motion.model_frames.at(-1)!) !==
        signature(physicalEndpointSources(family.physical.applied))
      )
        fail("terminal model frame differs from applied bindings");
    }
  }
}

function validateElementClip(
  contract: StateDeliveryContract,
  family: StateDeliveryContract["families"][number],
  motion: PhysicalElementClip,
) {
  const fail = (message: string): never => {
    throw new Error(`State delivery: ${message}`);
  };
  const hash = (v: unknown) => typeof v === "string" && /^[a-f0-9]{64}$/.test(v);
  const exactKeys = (value: object, keys: string[]) =>
    JSON.stringify(Object.keys(value).sort()) === JSON.stringify([...keys].sort());
  const element = contract.native.elements.find((e) => e.id === motion.element_id);
  const applied = family.physical.applied;
  if (
    !exactKeys(motion, [
      "kind",
      "element_id",
      "source_motion_sha256",
      "start_frame",
      "binding",
      "clip",
      "duration_ticks",
    ]) ||
    !hash(motion.source_motion_sha256) ||
    family.element_ids.length !== 1 ||
    family.element_ids[0] !== motion.element_id ||
    family.patch_ids?.length ||
    family.hidden_initial_element_ids?.length ||
    !element?.frames.length ||
    element.loop ||
    !Array.isArray(applied) ||
    applied.length !== 1
  )
    fail("invalid reviewed target clip binding");
  if (
    !Number.isSafeInteger(motion.start_frame) ||
    motion.start_frame <= 0 ||
    motion.start_frame >= element!.frames.length
  )
    fail("target clip start frame is outside the source frames");
  const start = nativeElementFrameStartTick(element!.frames, motion.start_frame);
  if (
    !Number.isSafeInteger(motion.duration_ticks) ||
    motion.duration_ticks <= 0 ||
    start + motion.duration_ticks !== family.body_terminal_tick
  )
    fail("target clip timing differs from source frames");
  if (typeof motion.clip !== "string" || !motion.clip.trim()) fail("missing target clip name");
  const binding = motion.binding,
    endpoint = (applied as PhysicalStateBinding[])[0]!;
  if (
    !binding ||
    typeof binding !== "object" ||
    binding.id === endpoint.id ||
    binding.role !== endpoint.role ||
    JSON.stringify(binding.position ?? null) !== JSON.stringify(endpoint.position ?? null)
  )
    fail("target clip placement differs from applied model binding");
}

function validateNativeRemainder(
  contract: StateDeliveryContract,
  family: StateDeliveryContract["families"][number],
) {
  const fail = (message: string): never => {
    throw new Error(`State delivery: ${message}`);
  };
  const hash = (v: unknown) => typeof v === "string" && /^[a-f0-9]{64}$/.test(v);
  const exactKeys = (value: object, keys: string[]) =>
    JSON.stringify(Object.keys(value).sort()) === JSON.stringify([...keys].sort());
  const bindings = family.native_remainder;
  if (!Array.isArray(bindings) || !bindings.length) fail("invalid native remainder");
  // TODO: remainder frames paired with reviewed physical motion need a per-frame pose rule.
  if (family.physical.motion !== undefined)
    fail("native remainder requires a static physical pairing");
  const bound = new Set<string>();
  for (const binding of bindings!) {
    if (
      !binding ||
      typeof binding !== "object" ||
      !exactKeys(binding, ["element_id", "encoding", "drawn_values", "priority", "frames"]) ||
      typeof binding.element_id !== "string" ||
      !family.element_ids.includes(binding.element_id) ||
      bound.has(binding.element_id) ||
      binding.encoding !== "source-domain-v1" ||
      binding.priority !== "remainder-over-physical" ||
      !Array.isArray(binding.drawn_values) ||
      !binding.drawn_values.includes(NATIVE_REMAINDER_MASK_VALUES.native) ||
      binding.drawn_values.some(
        (v, i) =>
          (v !== NATIVE_REMAINDER_MASK_VALUES.native &&
            v !== NATIVE_REMAINDER_MASK_VALUES.ambiguous) ||
          (i > 0 && v <= binding.drawn_values[i - 1]!),
      )
    )
      fail("invalid native remainder binding");
    bound.add(binding.element_id);
    const element = contract.native.elements.find((e) => e.id === binding.element_id)!;
    if (
      !Array.isArray(binding.frames) ||
      binding.frames.length !== element.frames.length ||
      !binding.frames.some((frame) => frame !== null)
    )
      fail("native remainder frames differ from source frames");
    const paths = new Set<string>();
    for (const [index, frame] of binding.frames.entries()) {
      if (frame === null) continue;
      const source = element.frames[index]!;
      const mask = frame?.mask;
      if (
        !frame ||
        typeof frame !== "object" ||
        !exactKeys(frame, ["mask", "physical"]) ||
        !mask ||
        !exactKeys(mask, ["path", "sha256", "width", "height"]) ||
        !safeLibraryPath(mask.path) ||
        !hash(mask.sha256) ||
        mask.width !== source.width ||
        mask.height !== source.height ||
        paths.has(mask.path)
      )
        fail("invalid native remainder mask");
      paths.add(mask.path);
      if (
        frame.physical !== "applied" ||
        !Array.isArray(family.physical.applied) ||
        !family.physical.applied.length
      )
        fail("native remainder frame lacks a physical pairing");
    }
  }
}

/** The reviewed remainder frame for one source frame, or undefined for whole native artwork. */
export function nativeRemainderFrame(
  family: StateDeliveryContract["families"][number],
  elementId: string,
  frame: number,
): (NativeRemainderFrame & { binding: NativeRemainderBinding }) | undefined {
  const binding = family.native_remainder?.find((b) => b.element_id === elementId);
  const selected = binding?.frames[frame];
  return binding && selected ? { ...selected, binding } : undefined;
}

/** Every independently loaded physical phase, including discrete transition models. */
export function physicalFamilyEndpoints(
  family: StateDeliveryContract["families"][number],
): PhysicalEndpoint[] {
  const { initial, applied, motion } = family.physical;
  return [
    initial,
    applied,
    ...(motion?.kind === "native-patch-model-frames" ? motion.model_frames : []),
    ...(motion?.kind === "native-patch-clips" ? [[motion.transition.binding]] : []),
    ...(motion?.kind === "native-element-clip" ? [[motion.binding]] : []),
  ];
}

/** Sample the native presentation timeline; callers own the clock and reset state. */
export function physicalPatchPose(
  contract: StateDeliveryContract,
  id: string,
  tick?: number,
): {
  endpoint: "initial" | "applied";
  translation: [number, number, number];
  node_frame?: number;
  model_frame?: number;
  visible?: boolean;
  clip_phase?: "transition" | "final";
  clip_tick?: number;
  /** The family's target still draws its native frame; no physical model is shown. */
  native_element?: boolean;
} {
  const family = contract.families.find((f) => f.id === id);
  const motion = family?.physical.motion;
  if (!family || !motion) throw new Error(`Missing physical motion: ${id}`);
  if (tick !== undefined && (!Number.isSafeInteger(tick) || tick < 0))
    throw new Error("Invalid physical motion tick");
  if (tick === undefined)
    return { endpoint: "initial" as const, translation: [0, 0, 0] as [number, number, number] };
  if (motion.kind === "native-element-stage-poses")
    throw new Error("Staged motion requires independent source clocks");
  if (motion.kind === "native-patch-clips")
    return physicalPatchClipPose(contract, id, "forward", tick);
  if (motion.kind === "native-element-clip") {
    const element = contract.native.elements.find((e) => e.id === motion.element_id)!;
    if (tick < nativeElementFrameStartTick(element.frames, motion.start_frame))
      return { endpoint: "applied", translation: [0, 0, 0], visible: false, native_element: true };
    if (tick >= family.body_terminal_tick) return { endpoint: "applied", translation: [0, 0, 0] };
    return {
      endpoint: "applied",
      translation: [0, 0, 0],
      model_frame: 0,
      clip_phase: "transition",
      clip_tick: tick,
    };
  }
  if (tick >= family.body_terminal_tick)
    return { endpoint: "applied" as const, translation: [0, 0, 0] as [number, number, number] };
  const frames =
    motion.kind === "native-element-node-poses"
      ? contract.native.elements.find((e) => e.id === motion.element_id)!.frames
      : contract.native.patch_states!.find((p) => p.id === motion.patch_id)!.transition;
  const frame = nativePresentationFrame({ frames, loop: false }, tick);
  if (motion.kind === "native-patch-node-poses" || motion.kind === "native-element-node-poses")
    return { endpoint: "applied", translation: [0, 0, 0], node_frame: frame };
  if (motion.kind === "native-patch-model-frames")
    return { endpoint: "initial", translation: [0, 0, 0], model_frame: frame };
  const translation = motion.translation_frames[frame];
  if (!translation) throw new Error("Missing reviewed physical pose");
  return { endpoint: "initial" as const, translation };
}

/** Applied events carry a fresh final-animation clock, never a transition cursor. */
export function physicalPatchClipPose(
  contract: StateDeliveryContract,
  id: string,
  phase: "initial" | "forward" | "applied" | "reverse",
  tick: number,
): ReturnType<typeof physicalPatchPose> {
  const family = contract.families.find((f) => f.id === id);
  const motion = family?.physical.motion;
  if (motion?.kind !== "native-patch-clips" || !Number.isSafeInteger(tick) || tick < 0)
    throw new Error("Invalid patch clip clock");
  if (phase === "initial") return { endpoint: "initial", translation: [0, 0, 0] };
  if (phase === "reverse") throw new Error("Unreviewed reverse patch clip");
  const terminal = Math.max(1, motion.transition.duration_ticks - 1);
  if (phase === "forward" && tick < terminal)
    return {
      endpoint: "initial",
      translation: [0, 0, 0],
      model_frame: 0,
      clip_phase: "transition",
      clip_tick: tick,
    };
  const finalTick = phase === "forward" ? tick - terminal : tick;
  return "kind" in motion.final
    ? { endpoint: "applied", translation: [0, 0, 0], visible: false }
    : {
        endpoint: "applied",
        translation: [0, 0, 0],
        clip_phase: "final",
        clip_tick: finalTick % motion.final.duration_ticks,
      };
}

export interface PhysicalStageClock {
  element_id: string;
  active: boolean;
  visible?: boolean;
  tick: number;
}

/** Select one complete assembly from independent clocks; never extrapolate a retired stage. */
export function physicalElementStagePose(
  contract: StateDeliveryContract,
  familyId: string,
  samples: readonly PhysicalStageClock[],
): ReturnType<typeof physicalPatchPose> {
  const family = contract.families.find((f) => f.id === familyId);
  const motion = family?.physical.motion;
  if (motion?.kind !== "native-element-stage-poses")
    throw new Error("Missing staged target motion");
  if (
    new Set(samples.map((s) => s.element_id)).size !== samples.length ||
    samples.some(
      (s) =>
        !motion.stages.some((stage) => stage.element_id === s.element_id) ||
        typeof s.active !== "boolean" ||
        (s.visible !== undefined && typeof s.visible !== "boolean") ||
        !Number.isSafeInteger(s.tick) ||
        s.tick < 0,
    )
  )
    throw new Error("Invalid staged owner clocks");
  let active = false,
    selected: number | undefined,
    offset = 0;
  for (const stage of motion.stages) {
    const sample = samples.find((s) => s.element_id === stage.element_id);
    const frames = contract.native.elements.find((e) => e.id === stage.element_id)!.frames;
    if (sample?.active) {
      active = true;
      const index = nativePresentationFrame({ frames, loop: false }, sample.tick);
      if (sample.visible !== false && stage.node_frames[index] !== null) {
        if (selected !== undefined) throw new Error("Overlapping visible assembly stages");
        selected =
          offset + stage.node_frames.slice(0, index).filter((frame) => frame !== null).length;
      }
    }
    offset += stage.node_frames.filter((frame) => frame !== null).length;
  }
  if (!active)
    return {
      endpoint: "initial",
      translation: [0, 0, 0],
      visible: motion.stages.some(
        (stage) => samples.find((s) => s.element_id === stage.element_id)?.visible !== false,
      ),
    };
  return {
    endpoint: "applied",
    translation: [0, 0, 0],
    node_frame: selected,
    visible: selected !== undefined,
  };
}

/** Compiled movement transitions as emitted in a compiled map's asset geometry. */
export interface CompiledGameplayTransitions {
  movement_transitions?: readonly {
    id: string;
    aliases?: readonly string[];
    definitive: boolean;
    initial_sight?: readonly number[];
    applied_sight?: readonly number[];
  }[];
  sight_obstacles?: readonly unknown[];
}

/** One verified state-to-gameplay link with the compiled sight obstacles of each endpoint. */
export interface StateGameplayLink {
  family: string;
  transition_id: string;
  definitive: boolean;
  initial_sight: number[];
  applied_sight: number[];
}

/** Resolve every linked transition in one compiled map. Missing compiled data is an error. */
export function verifyStateGameplayLinks(
  contract: StateDeliveryContract,
  compiled: CompiledGameplayTransitions | undefined,
  warnings: readonly string[],
): StateGameplayLink[] {
  validateStateDelivery(contract);
  const linked = contract.families.filter((f) => f.gameplay_transition_ids?.length);
  if (!linked.length) return [];
  const transitions = compiled?.movement_transitions;
  const sight = compiled?.sight_obstacles;
  if (!Array.isArray(transitions) || !Array.isArray(sight) || !Array.isArray(warnings))
    throw new Error("Gameplay link requires compiled movement transitions and sight obstacles");
  const links: StateGameplayLink[] = [];
  for (const family of linked) {
    const states = [
      ...(contract.native.background_states ?? []).filter((s) =>
        family.background_ids.includes(s.id),
      ),
      ...(contract.native.patch_states ?? []).filter((s) => family.patch_ids?.includes(s.id)),
    ];
    for (const id of family.gameplay_transition_ids!) {
      const matches = transitions.filter((t) => t.id === id || t.aliases?.includes(id));
      if (matches.length !== 1)
        throw new Error(`Gameplay link does not resolve to one compiled transition: ${id}`);
      const transition = matches[0]!;
      if (
        warnings.some(
          (w) =>
            typeof w === "string" &&
            [id, transition.id].some((t) => w.startsWith(`Transition ${t}: control omitted`)),
        )
      )
        throw new Error(`Gameplay link names an omitted compiled transition: ${id}`);
      if (
        typeof transition.definitive !== "boolean" ||
        states.some((s) => s.definitive !== transition.definitive)
      )
        throw new Error(`Gameplay link permanence differs from its bound state: ${id}`);
      const indices = (v: readonly number[] | undefined) => {
        const rows = [...(v ?? [])];
        if (rows.some((i) => !Number.isSafeInteger(i) || i < 0 || i >= sight.length))
          throw new Error(`Gameplay link sight obstacle is outside the compiled map: ${id}`);
        return rows;
      };
      links.push({
        family: family.id,
        transition_id: transition.id,
        definitive: transition.definitive,
        initial_sight: indices(transition.initial_sight),
        applied_sight: indices(transition.applied_sight),
      });
    }
  }
  return links;
}

/** Reject edited or unrelated placements before any visible node is suppressed. */
export function verifyStaticStateReplacements(
  contract: StateDeliveryContract,
  document: Level3D,
): void {
  validateStateDelivery(contract);
  verifyStaticReplacementRows(
    contract.families.flatMap((f) => Object.values(f.static_replacements ?? {}).flat()),
    document,
  );
}

export function validateStaticReplacementRows(
  rows: unknown,
): asserts rows is StaticStateReplacement[] {
  const fail = (reason: string): never => {
    throw new Error(reason);
  };
  const hash = (v: unknown) => typeof v === "string" && /^[a-f0-9]{64}$/.test(v);
  const transform = (t: unknown): t is GameTransform => {
    if (!t || typeof t !== "object") return false;
    const value = t as GameTransform;
    return [value.dx, value.dy, value.dz, value.rot_deg].every(Number.isFinite);
  };
  if (!Array.isArray(rows)) throw new Error("invalid static replacements");
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
        (typeof row.group_id !== "string" || !row.group_id || !transform(row.group_transform)))
    )
      fail("invalid static replacement");
    objects.add(row.object_id);
  }
}

/** Verify exact static receiver identities shared by endpoint and loop previews. */
export function verifyStaticReplacementRows(
  rows: readonly StaticStateReplacement[],
  document: Level3D,
): void {
  const sameTransform = (a: GameTransform, b: GameTransform) =>
    a.dx === b.dx && a.dy === b.dy && a.dz === b.dz && a.rot_deg === b.rot_deg;
  for (const row of rows) {
    const object = document.objects.find((o) => o.id === row.object_id);
    const asset = document.assetSources?.find((a) => a.id === row.asset_id);
    const group =
      row.group_id === undefined ? undefined : document.groups.find((g) => g.id === row.group_id);
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
      throw new Error(`State replacement differs from the displayed placement: ${row.object_id}`);
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

/** Source artwork loops retain independent clocks and may include immutable startup context. */
export interface NativeLoopPreviewContract {
  /** Freeze nonloop/state artwork at source tick0; grants no transition or patch activation. */
  initial_context?: NativeInitialContextMode;
  cover_visibility?: SourceCoverVisibility[];
  version: 1;
  scope: "controlled-native-loop-preview";
  native: NativeStatePresentationContract;
  focus_element_id: string;
  /** Optional separately pinned animated model delivery; never an endpoint asset. */
  physical?: MissionStateContract;
  /** Exact static parts suppressed only while the reviewed physical loop is shown. */
  static_replacements?: StaticStateReplacement[];
}
export function validateNativeLoopPreview(
  value: unknown,
): asserts value is NativeLoopPreviewContract {
  if (!value || typeof value !== "object") throw new Error("Missing native loop preview");
  const contract = value as NativeLoopPreviewContract;
  if (contract.version !== 1 || contract.scope !== "controlled-native-loop-preview")
    throw new Error("Invalid native loop preview");
  if (contract.cover_visibility !== undefined) {
    validateSourceCoverVisibility(contract.cover_visibility);
    if (!contract.physical) throw new Error("Cover visibility requires a physical loop");
  }
  validateNativeStatePresentation(contract.native);
  if (
    contract.initial_context !== undefined &&
    contract.initial_context !== "immutable-source-initial"
  )
    throw new Error("Invalid native loop initial context");
  const focus = contract.native.elements.find((e) => e.id === contract.focus_element_id);
  if (
    !focus ||
    !focus.active ||
    !focus.loop ||
    !focus.frames.length ||
    (!contract.initial_context &&
      (contract.native.background_states?.length ||
        contract.native.patch_states?.length ||
        contract.native.elements.some((e) => e.frames.length && !e.loop)))
  )
    throw new Error("Native loop preview requires a visible looping focus and independent loops");
  if (contract.static_replacements !== undefined) {
    if (!contract.physical) throw new Error("Static loop replacements require physical delivery");
    validateStaticReplacementRows(contract.static_replacements);
  }
  if (contract.physical) {
    const p = contract.physical,
      n = contract.native;
    validateMissionStateContract(p);
    if (
      p.mission !== n.mission ||
      p.mission_data_sha256 !== n.mission_data_sha256 ||
      p.level_data_sha256 !== n.level_data_sha256 ||
      p.camera_elevation_deg !== n.camera_elevation_deg
    )
      throw new Error("Physical/native loop sources differ");
    const bound = new Set<string>();
    for (const target of p.targets) {
      const e = n.elements.find(
        (e) =>
          e.source.kind === (target.source_kind ?? "mission-target") &&
          e.source.index === target.target_index,
      );
      if (
        !e ||
        !e.loop ||
        !e.frames.length ||
        e.source.sha256 !== target.target_sha256 ||
        target.representation !== "physical"
      )
        throw new Error("Physical loop target lacks exact native source identity");
      const cycle = e.frames.reduce((sum, f) => sum + f.delay + 1, 0);
      if (target.actions.some((a) => a.timing.mode !== "loop" || a.timing.cycleTicks !== cycle))
        throw new Error("Physical/native loop cycle differs");
      bound.add(`${target.source_kind ?? "mission-target"}:${target.target_index}`);
    }
    if (
      p.targets.some((t) => t.source_kind !== "map-animation") &&
      n.elements.some(
        (e) =>
          e.source.kind === "mission-target" &&
          e.frames.length &&
          (e.loop || !contract.initial_context) &&
          !bound.has(`${e.source.kind}:${e.source.index}`),
      )
    )
      throw new Error("Missing physical loop target");
  }
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
